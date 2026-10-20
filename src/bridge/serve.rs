//! The read side of the protocol, shared by both peers: the host answers it when it
//! has decrypted the box itself, and the app answers it for the box it has open.
//!
//! Everything else — the file, the boxes, the lock, the writes — means something
//! different on each side, so the caller owns it and [`read`] hands it back. That is
//! why this module is a function set and not a trait: the two peers agree on what a
//! *question* means, not on how to change anything. The two writes the panel can ask
//! for still build the entry here ([`create_login`], [`update_login`]); what each
//! peer does with the changed vault afterwards is its own.

use crate::bridge::protocol::{Candidate, CodeInfo, Request, Response};
use crate::vault::matching;
use crate::vault::model::{now_secs, Login, Vault};
use crate::vault::totp;
use zeroize::Zeroizing;

/// The box behind a request. `Locked` is the absence of one, not an error.
#[derive(Clone, Copy)]
pub enum Peer<'a> {
    /// No box here. `unlockable` says whether this peer could open one if the user
    /// asked: the host can, the app cannot — opening a box is its window's job.
    Locked {
        unlockable: bool,
    },
    Open(&'a Vault),
}

/// Answer a request both peers answer the same way, or `None` when it is the
/// caller's own — see the module comment for which those are.
pub fn read(request: &Request, peer: Peer<'_>) -> Option<Response> {
    match request {
        Request::Status => Some(match peer {
            Peer::Locked { unlockable } => Response::Status {
                unlocked: false,
                unlockable,
                name: None,
                count: 0,
            },
            Peer::Open(vault) => Response::Status {
                unlocked: true,
                unlockable: false,
                name: Some(vault.name.clone()),
                count: vault.logins.len(),
            },
        }),

        Request::Entries => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => Response::Entries {
                entries: entries(vault),
            },
        }),

        Request::Match { host, url } => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => Response::Entries {
                entries: matches(
                    vault,
                    url.as_deref().or(host.as_deref()).unwrap_or_default(),
                ),
            },
        }),

        Request::Reveal { id } => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => match reveal(vault, id) {
                Some((username, password)) => Response::Secret { username, password },
                None => Response::Error {
                    message: "no such entry".to_owned(),
                },
            },
        }),

        Request::Codes => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => Response::Codes {
                codes: codes(vault),
            },
        }),

        Request::Code { id } => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => match code(vault, id, now_secs()) {
                Some((code, remaining)) => Response::Code {
                    code: code.to_string(),
                    remaining,
                },
                None => Response::Error {
                    message: "no such code".to_owned(),
                },
            },
        }),

        _ => None,
    }
}

/// Add a login the panel saved. The vault is changed here; how the change reaches
/// the disk is the peer's own (the host saves, the app marks the box dirty).
pub fn create_login(
    vault: &mut Vault,
    name: String,
    username: String,
    password: &str,
    url: String,
) -> Candidate {
    let mut item = Login::new(now_secs());
    item.name = name;
    item.username = username;
    item.password = Zeroizing::new(password.to_owned());
    item.url = url;
    let entry = candidate(&item, (!item.url.is_empty()).then_some(item.url.as_str()));
    vault.logins.push(item);
    entry
}

/// Change the fields the panel sent; anything it left out keeps what it had.
/// `None` when no login carries the id.
pub fn update_login(
    vault: &mut Vault,
    id: &str,
    username: Option<String>,
    password: Option<&str>,
) -> Option<Candidate> {
    let item = vault.logins.iter_mut().find(|item| item.id == id)?;
    if let Some(username) = username {
        item.username = username;
    }
    if let Some(password) = password {
        item.password = Zeroizing::new(password.to_owned());
    }
    item.touched(now_secs());
    Some(candidate(
        item,
        (!item.url.is_empty()).then_some(item.url.as_str()),
    ))
}

/// Every entry, for the popup's own browsing — never a password.
pub fn entries(vault: &Vault) -> Vec<Candidate> {
    vault
        .logins
        .iter()
        .map(|item| candidate(item, (!item.url.is_empty()).then_some(item.url.as_str())))
        .collect()
}

/// Entries that claim the page the user is on, the most specific match first, each
/// naming the URL that matched it. An entry with no URL never matches.
pub fn matches(vault: &Vault, page: &str) -> Vec<Candidate> {
    let Some(page) = matching::parse(page) else {
        return Vec::new();
    };
    let mut hits: Vec<(&Login, matching::Rank, &str)> = vault
        .logins
        .iter()
        .filter_map(|item| matching::rank(item, &page).map(|(rank, url)| (item, rank, url)))
        .collect();
    // A more specific rule first, then by name so the order never wobbles between
    // two calls that saw the same vault and the same page.
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    hits.into_iter()
        .map(|(item, _, url)| candidate(item, Some(url)))
        .collect()
}

/// The one place a password is put into a response, and only for the id the user
/// picked.
pub fn reveal(vault: &Vault, id: &str) -> Option<(String, String)> {
    vault
        .logins
        .iter()
        .find(|item| item.id == id)
        .map(|item| (item.username.clone(), item.password.to_string()))
}

/// Every seed in the box, by name. The seed never leaves it, and neither does a
/// code: this is the listing, and what it carries is what a `Candidate` carries.
pub fn codes(vault: &Vault) -> Vec<CodeInfo> {
    vault
        .otps
        .iter()
        .map(|otp| CodeInfo {
            id: otp.id.clone(),
            name: otp.name.clone(),
        })
        .collect()
}

/// The code a seed is holding at `now`, and how long it has left. Derived on the
/// spot and for one id only — the same bargain `reveal` makes, because a code is to
/// a seed what a password is to a login, and a page needs the first, never the
/// second.
pub fn code(vault: &Vault, id: &str, now: u64) -> Option<(Zeroizing<String>, u64)> {
    let otp = vault.otps.iter().find(|otp| otp.id == id)?;
    totp::at(&otp.secret, now).ok()
}

pub fn candidate(item: &Login, uri: Option<&str>) -> Candidate {
    Candidate {
        id: item.id.clone(),
        name: item.name.clone(),
        username: item.username.clone(),
        uri: uri.map(str::to_owned),
    }
}

/// The one error every locked request gets. The extension turns it into "click the
/// toolbar icon to unlock".
pub fn locked() -> Response {
    Response::Error {
        message: "locked".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::Otp;

    const SEED: &str = "JBSWY3DPEHPK3PXP";

    fn boxed() -> Vault {
        let mut vault = Vault::new("Box");
        let mut otp = Otp::new(1);
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new(SEED.to_owned());
        vault.otps.push(otp);
        vault
    }

    /// The listing is the shape a `Candidate` has: an id and a name, never a secret.
    #[test]
    fn listing_codes_carries_neither_the_seed_nor_a_code() {
        let vault = boxed();
        let json =
            serde_json::to_string(&read(&Request::Codes, Peer::Open(&vault)).unwrap()).unwrap();
        assert!(json.contains("GitHub"), "{json}");
        assert!(!json.contains(SEED), "the seed stays in the box: {json}");
        let code = super::code(&vault, &vault.otps[0].id, 1_700_000_000).unwrap();
        assert!(
            !json.contains(&*code.0),
            "a listing carries no code: {json}"
        );
    }

    /// What goes back is exactly what the seed derives at that second, and only for
    /// the id that was asked for.
    #[test]
    fn one_code_is_derived_for_one_id_at_a_time() {
        let vault = boxed();
        let (expected, left) = totp::at(SEED, 1_700_000_000).unwrap();
        let (derived, remaining) = code(&vault, &vault.otps[0].id, 1_700_000_000).unwrap();
        assert_eq!(derived, expected);
        assert_eq!(remaining, left);

        // Through the protocol the clock is the real one, so what is checked there is
        // the shape of the answer and the one way it fails — never a second guess.
        let answer = read(
            &Request::Code {
                id: vault.otps[0].id.clone(),
            },
            Peer::Open(&vault),
        )
        .unwrap();
        match answer {
            Response::Code { code, remaining } => {
                assert_eq!(code.len(), 6, "a six-digit code by default: {code}");
                assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
                assert!((1..=30).contains(&remaining), "{remaining}");
            }
            other => panic!("{other:?}"),
        }

        // An id nobody has is an error, not an empty code.
        let answer = read(
            &Request::Code {
                id: "no such seed".to_owned(),
            },
            Peer::Open(&vault),
        )
        .unwrap();
        assert!(matches!(answer, Response::Error { .. }), "{answer:?}");
    }

    /// A locked peer answers every read the same way, codes included.
    #[test]
    fn a_locked_peer_carries_no_code() {
        let locked = Peer::Locked { unlockable: false };
        for request in [Request::Codes, Request::Code { id: "x".to_owned() }] {
            assert!(matches!(
                read(&request, locked).unwrap(),
                Response::Error { .. }
            ));
        }
    }
}
