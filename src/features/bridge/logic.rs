//! Answering for the box the browser reads from.
//!
//! Same protocol as the host, different answers for the few requests that are about
//! *owning* a vault: the app picks the box, opens it, locks it and writes it, so the
//! extension is told to look at the window for those, and the reads come out of the
//! vault that is already decrypted here. The browser may read from any box the app
//! has open: the one on screen until it picks another, and a page's fill spans every
//! one of them, each row saying which box it came from.

use crate::app::{App, StatusKind};
use crate::bridge::protocol::{BoxInfo, Candidate, Request, Response};
use crate::bridge::serve::{self, Peer};
use crate::features::boxes::model::Boxes;
use crate::features::generator::logic as generator;
use std::path::{Path, PathBuf};

use super::model::Effect;

/// Every open box's entries, each tagged with the box it came from: a page has no
/// idea which box holds what, so all of them answer, and the row says which.
fn candidates(boxes: &mut Boxes, page: Option<&str>) -> Vec<Candidate> {
    boxes
        .scan(|path, vault| {
            let found = match page {
                Some(page) => serve::matches(vault, page),
                None => serve::entries(vault),
            };
            let path = path.display().to_string();
            let name = vault.name.clone();
            found
                .into_iter()
                .map(|entry| Candidate {
                    box_path: Some(path.clone()),
                    box_name: Some(name.clone()),
                    ..entry
                })
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()
}

/// Which box the browser reads from, as the listing sends it back: the pick while that
/// box is still open, else the box on screen — the same fallback a request takes, so the
/// panel never names a box the app would not answer from.
fn reading(boxes: &Boxes, picked: &Option<PathBuf>) -> Option<String> {
    let still_open = picked
        .as_deref()
        .filter(|path| boxes.position(path).is_some());
    still_open
        .or_else(|| boxes.open().first().map(|(path, _)| *path))
        .map(|path| path.display().to_string())
}

/// Read from another open box.
fn select(path: String, boxes: &mut Boxes, picked: &mut Option<PathBuf>) -> Response {
    let path = PathBuf::from(path);
    if boxes.position(&path).is_none() {
        return Response::Error {
            message: "that box is not open".to_owned(),
        };
    }
    *picked = Some(path);
    Response::Ok
}

/// Answer one request. `boxes` are the boxes the app has open and `picked` the one the
/// browser chose among them, if it did.
pub fn answer(request: Request, boxes: &mut Boxes, picked: &mut Option<PathBuf>) -> Response {
    // The requests about which box to read come first: the rest are answered off one.
    let request = match request {
        Request::ListBoxes => {
            let open = boxes.open();
            return Response::Boxes {
                boxes: open
                    .into_iter()
                    .map(|(path, name)| BoxInfo {
                        path: path.display().to_string(),
                        name: name.to_owned(),
                    })
                    .collect(),
                picked: self::reading(boxes, picked),
            };
        }
        Request::Select { path } => return select(path, boxes, picked),
        // Opening a box is the app window's job, whatever the browser asks.
        Request::Unlock { .. } => {
            return Response::Error {
                message: "opening a box happens in the Pandora window".to_owned(),
            }
        }
        // A page's listing spans every open box, each row naming its own. With none
        // open the answer is the one a locked box gives, not an empty list.
        Request::Entries => {
            if boxes.open().is_empty() {
                return serve::locked();
            }
            return Response::Entries {
                entries: candidates(boxes, None),
            };
        }
        Request::Match { host, url } => {
            if boxes.open().is_empty() {
                return serve::locked();
            }
            let page = url.as_deref().or(host.as_deref()).unwrap_or_default();
            return Response::Entries {
                entries: candidates(boxes, Some(page)),
            };
        }
        other => other,
    };

    // A row listed from another box comes back named: the entry's id alone would land
    // on whichever box happens to read next.
    let named = match &request {
        Request::Reveal { box_path, .. } | Request::Update { box_path, .. } => {
            box_path.as_deref().map(Path::new)
        }
        _ => None,
    };
    let session = match boxes.pick(named.or(picked.as_deref())) {
        Ok(session) => session,
        Err(message) => return Response::Error { message },
    };

    match request {
        // The app owns the file, so it is never stale in the way the host is.
        Request::Refresh => match session {
            Some(session) => Response::Unlocked {
                name: session.vault.name.clone(),
                count: session.vault.logins.len(),
            },
            None => serve::locked(),
        },

        Request::Create {
            name,
            username,
            password,
            url,
        } => {
            let Some(session) = session else {
                return serve::locked();
            };
            let entry = serve::create_login(&mut session.vault, name, username, &password, url);
            // The app's own autosave writes it: one writer, no conflict to detect.
            session.mark_now();
            Response::Saved { entry }
        }

        Request::Update {
            id,
            username,
            password,
            ..
        } => {
            let Some(session) = session else {
                return serve::locked();
            };
            let Some(entry) =
                serve::update_login(&mut session.vault, &id, username, password.as_deref())
            else {
                return Response::Error {
                    message: "no such entry".to_owned(),
                };
            };
            session.mark_now();
            Response::Saved { entry }
        }

        Request::PasskeyCreate { rp_id, user_name } => {
            let Some(session) = session else {
                return serve::locked();
            };
            let response = crate::host::passkeys::create_in(&mut session.vault, &rp_id, &user_name);
            if !matches!(response, Response::Error { .. }) {
                session.mark_now();
            }
            response
        }

        Request::PasskeyGet {
            rp_id,
            credential_id,
            challenge,
            client_data,
        } => {
            let Some(session) = session else {
                return serve::locked();
            };
            let (Ok(id), Ok(data)) = (
                crate::vault::codec::decode(&credential_id),
                crate::vault::codec::decode(&client_data),
            ) else {
                return Response::Error {
                    message: "the page sent something unreadable".to_owned(),
                };
            };
            if !String::from_utf8_lossy(&data).contains(&challenge) {
                return Response::Error {
                    message: "the client data is not for this challenge".to_owned(),
                };
            }
            let response = crate::host::passkeys::assert_in(&mut session.vault, &rp_id, &id, &data);
            if !matches!(response, Response::Error { .. }) {
                session.mark_now();
            }
            response
        }

        // Locking is the app's: it clears the session, which is what drops the keys.
        // The pump reports it to the app, so this only ever answers a caller that
        // reached it some other way.
        Request::Lock => Response::Error {
            message: "lock the Pandora window instead".to_owned(),
        },

        // Pure, and nothing is stored here: a fresh password for a signup form. The
        // host answers this the same way, so it must not fall through as unsupported.
        Request::Generate { length } => Response::Generated {
            password: generator::roll(length),
        },

        // Everything both peers answer the same way, read off the box on screen.
        other => {
            let peer = match session.as_deref() {
                Some(session) => Peer::Open(&session.vault),
                None => Peer::Locked { unlockable: false },
            };
            serve::read(&other, peer).unwrap_or_else(|| Response::Error {
                // Never the request back: an unknown shape can carry a password,
                // and an error is the last place it should leave the box's side.
                message: crate::shared::i18n::t(
                    "the extension asked for something Pandora does not serve",
                )
                .to_owned(),
            })
        }
    }
}

impl App {
    /// A frame of the bridge, from the app's loop: answer what the extension asked,
    /// and lock when it asked for that.
    pub fn pump_bridge(&mut self, ctx: &egui::Context) {
        let effect = self.bridge.pump(&mut self.boxes, &self.prefs);
        match effect {
            Effect::Lock => {
                self.lock(ctx);
                self.shell
                    .set_status("locked from the browser", StatusKind::Info);
            }
            Effect::Wrote => {
                self.shell.activity = Some(std::time::Instant::now());
                self.shell
                    .set_status("saved from the browser", StatusKind::Saved);
            }
            Effect::Activity => self.shell.activity = Some(std::time::Instant::now()),
            Effect::None => {}
        }
    }
}
