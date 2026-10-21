//! The stdio host the browser launches. Every secret operation goes through `vault/`,
//! so the extension never learns the format or the crypto; it draws nothing.

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

/// The extension turns it into a sentence when the user has not allowed a browser to open a box.
const UNLOCK_OFF: &str = "unlock this box in the Pandora app";

/// Serve requests on stdio until the browser closes the port.
pub fn run() -> std::io::Result<()> {
    // A flatpak remaps the home dir; the install points the host at the real data dir.
    if let Some(dir) = std::env::var_os("PANDORA_DATA_DIR").filter(|dir| !dir.is_empty()) {
        crate::vault::store::set_data_dir(PathBuf::from(dir));
    }

    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    let mut current: Option<Session> = None;
    let caller = caller();
    // Read once: whether a browser may open a box decides how this host answers.
    let prefs = crate::features::settings::model::load();

    while let Some(bytes) = protocol::read(&mut input)? {
        let envelope = protocol::parse(&bytes);
        // The app first when present: it already holds the key for the open box.
        let response = match relay(&bytes, &envelope, &caller) {
            Some(response) => response,
            None => match envelope.request {
                Ok(request) => handle(&mut current, request, &prefs),
                Err(message) => Response::Error { message },
            },
        };

        // `_id` rides back so the extension pairs a reply with its call.
        protocol::reply(&mut output, envelope.id, &response)?;
    }
    Ok(())
}

/// Who the browser says is calling, from our own argv: Chromium passes the origin,
/// Firefox the add-on id. The browser's word, not the caller's, is what the user approves.
fn caller() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [origin, ..] if origin.starts_with("chrome-extension://") => origin.clone(),
        [_, id, ..] => id.clone(),
        _ => "unknown".to_owned(),
    }
}

/// Hand the message to the app, or `None` with no bridge. Two requests never leave here:
/// listing boxes (the recents are ours) and unlocking (the window's job).
fn relay(bytes: &[u8], envelope: &Envelope, caller: &str) -> Option<Response> {
    let Ok(request) = &envelope.request else {
        return None;
    };
    if matches!(request, Request::ListBoxes | Request::Unlock { .. }) {
        return None;
    }

    let mut stream = socket::connect().ok()?;
    // The app cannot see the browser's process table, so it is told what the browser said.
    let mut message: Value = serde_json::from_slice(bytes).ok()?;
    message
        .as_object_mut()?
        .insert("caller".to_owned(), Value::from(caller));
    protocol::write(&mut stream, &serde_json::to_vec(&message).ok()?).ok()?;

    // The wait must outlast the app's approval prompt.
    stream
        .set_read_timeout(Some(Duration::from_secs(35)))
        .ok()?;
    let reply = protocol::read(&mut stream).ok()??;
    serde_json::from_slice(&reply).ok()
}

fn handle(current: &mut Option<Session>, request: Request, prefs: &Prefs) -> Response {
    match request {
        // Listing and unlocking are the window's business unless the user allowed browser unlock.
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
                    // The file's name: the box's own name is inside, and only an unlocked session has it.
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

        // Writes are unlocked-only and name fields, never a path.
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
                // Never echo the request: an unknown shape can carry a password, and an
                // error is the last place it should come back out.
                message: crate::shared::i18n::t(
                    "the extension asked for something Pandora does not serve",
                )
                .to_owned(),
            })
        }
    }
}
