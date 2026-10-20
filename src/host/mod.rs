//! The browser extension's end of the native-messaging channel: a stdio host the
//! browser launches. It reuses `vault/` for every secret operation, so the
//! extension never learns the file format or the crypto — it asks for a site's
//! entries and, once the user picks one, for that one password.
//!
//! A driver, like `app`: it composes `vault/` and the recents list and draws
//! nothing. Run it with `pandora-native-host` (see `src/bin/`).

mod session;

use crate::bridge::protocol::{self, BoxInfo, Envelope, Request, Response};
use crate::bridge::serve::Peer;
use crate::bridge::{serve, socket};
use crate::features::auth::model as recents;
use crate::features::generator::logic as generator;
use crate::features::settings::model::Prefs;
use serde_json::Value;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;

use session::Session;

/// What the extension is told when the user has not allowed a browser to open a box
/// of its own. It is the extension's job to turn this into a sentence.
const UNLOCK_OFF: &str = "unlock this box in the Pandora app";

/// Serve requests on stdio until the browser closes the port.
pub fn run() -> std::io::Result<()> {
    // A flatpak package remaps the home dir, so the app's own data dir — the
    // recents list and the default box — sits somewhere else inside the sandbox.
    // The install points the host at the real one through the environment.
    if let Some(dir) = std::env::var_os("PANDORA_DATA_DIR").filter(|dir| !dir.is_empty()) {
        crate::vault::store::set_data_dir(PathBuf::from(dir));
    }

    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    let mut current: Option<Session> = None;
    let caller = caller();
    // Read once: whether a browser may open a box of its own is the whole difference
    // between the two ways this host can answer.
    let prefs = crate::features::settings::model::load();

    while let Some(bytes) = protocol::read(&mut input)? {
        let envelope = protocol::parse(&bytes);
        // The app first, when it is there: it is the one holding the key for the box
        // the user already has open, which is the whole point of the bridge.
        let response = match relay(&bytes, &envelope, &caller) {
            Some(response) => response,
            None => match envelope.request {
                Ok(request) => handle(&mut current, request, &prefs),
                Err(message) => Response::Error { message },
            },
        };

        // `_id` rides back so the extension can pair a reply with its call: a
        // native-messaging port is one stream with no request/response framing.
        protocol::reply(&mut output, envelope.id, &response)?;
    }
    Ok(())
}

/// Who the browser says is calling, out of our own argv.
///
/// Chromium passes the caller's origin as the only argument; Firefox passes the path
/// of this host's own manifest and then the add-on id. It is the browser's word
/// rather than the caller's, which is the point — it is the name the user approves in
/// the app, and the caller does not get to choose it.
fn caller() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [origin, ..] if origin.starts_with("chrome-extension://") => origin.clone(),
        [_, id, ..] => id.clone(),
        _ => "unknown".to_owned(),
    }
}

/// Hand the message to the app and bring back its answer, or `None` when there is no
/// bridge to hand it to.
///
/// Two requests never leave here: which boxes exist (the recents list is ours to
/// read), and unlocking — the app already has a box open, and its own way to open
/// another.
fn relay(bytes: &[u8], envelope: &Envelope, caller: &str) -> Option<Response> {
    let Ok(request) = &envelope.request else {
        return None;
    };
    if matches!(request, Request::ListBoxes | Request::Unlock { .. }) {
        return None;
    }

    let mut stream = socket::connect().ok()?;
    // The app cannot see the browser's process table, so it is told what the browser
    // said.
    let mut message: Value = serde_json::from_slice(bytes).ok()?;
    message
        .as_object_mut()?
        .insert("caller".to_owned(), Value::from(caller));
    protocol::write(&mut stream, &serde_json::to_vec(&message).ok()?).ok()?;

    // The app may be waiting on the user to approve this browser, so the wait has to
    // outlast that prompt.
    stream
        .set_read_timeout(Some(Duration::from_secs(35)))
        .ok()?;
    let reply = protocol::read(&mut stream).ok()??;
    serde_json::from_slice(&reply).ok()
}

fn handle(current: &mut Option<Session>, request: Request, prefs: &Prefs) -> Response {
    match request {
        // Listing boxes and opening one are the window's business unless the user has
        // said a browser may open a box of its own. Off by default: with the app open
        // the host relays to it and there is nothing to ask for.
        Request::ListBoxes | Request::Unlock { .. } if !prefs.unlock_from_browser => {
            Response::Error {
                message: UNLOCK_OFF.to_owned(),
            }
        }

        Request::ListBoxes => Response::Boxes {
            boxes: recents::load()
                .into_iter()
                .map(|recent| BoxInfo {
                    path: recent.path.to_string_lossy().into_owned(),
                    // What the list calls it: the file's name. The box's own name
                    // is inside the box, and only an unlocked session has it.
                    name: recent.label(),
                })
                .collect(),
        },

        Request::Unlock { path, password } => {
            let password = Zeroizing::new(password);
            match Session::unlock(PathBuf::from(path), &password) {
                Ok(session) => {
                    let response = Response::Unlocked {
                        name: session.name().to_owned(),
                        count: session.count(),
                    };
                    *current = Some(session);
                    response
                }
                Err(message) => Response::Error { message },
            }
        }

        Request::Lock => {
            *current = None;
            Response::Ok
        }

        // Writing is unlocked-only and goes to the box the host already holds:
        // the extension names fields, never a path.
        Request::Create {
            name,
            username,
            password,
            url,
        } => match current {
            Some(session) => {
                let secret = Zeroizing::new(password);
                match session.create(name, username, &secret, url) {
                    Ok(entry) => Response::Saved { entry },
                    Err(message) => Response::Error { message },
                }
            }
            None => serve::locked(),
        },

        Request::Update {
            id,
            username,
            password,
        } => match current {
            Some(session) => {
                let secret = password.map(Zeroizing::new);
                match session.update(&id, username, secret.as_ref().map(|p| p.as_str())) {
                    Ok(entry) => Response::Saved { entry },
                    Err(message) => Response::Error { message },
                }
            }
            None => serve::locked(),
        },

        // Pure: generating needs no box open, and nothing is stored here.
        Request::Generate { length } => Response::Generated {
            password: generator::roll(length),
        },

        Request::Refresh => match current {
            Some(session) => match session.refresh() {
                Ok(()) => Response::Unlocked {
                    name: session.name().to_owned(),
                    count: session.count(),
                },
                Err(message) => Response::Error { message },
            },
            None => serve::locked(),
        },

        // Everything both peers answer the same way, read off the box we hold.
        other => {
            let peer = match current.as_ref() {
                Some(session) => Peer::Open(session.vault()),
                None => Peer::Locked {
                    unlockable: prefs.unlock_from_browser,
                },
            };
            serve::read(&other, peer).unwrap_or_else(|| Response::Error {
                // The request itself is not echoed: an unknown shape can carry
                // anything the caller put in it, a password included, and an
                // error is the last place it should come back out.
                message: crate::shared::i18n::t(
                    "the extension asked for something Pandora does not serve",
                )
                .to_owned(),
            })
        }
    }
}
