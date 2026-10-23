//! Answering for the box the app has open.
//!
//! Same protocol as the host, different answers for the few requests that are about
//! *owning* a vault: the app picks the box, opens it, locks it and writes it, so the
//! extension is told to look at the window for those, and the reads come out of the
//! vault that is already decrypted here.

use crate::app::{App, StatusKind};
use crate::bridge::protocol::{Request, Response};
use crate::bridge::serve::{self, Peer};
use crate::features::boxes::model::Session;
use crate::features::generator::logic as generator;

use super::model::Effect;

/// Answer one request. `session` is the box on screen, if any.
pub fn answer(request: Request, session: Option<&mut Session>) -> Response {
    match request {
        // Which boxes exist and how to open one are questions the app answers with
        // its own window; there is nothing for the extension to choose.
        Request::ListBoxes | Request::Unlock { .. } => Response::Error {
            message: "opening a box happens in the Pandora window".to_owned(),
        },

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
        let session = self.boxes.session.as_mut();
        let effect = self.bridge.pump(session, &self.prefs);
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
