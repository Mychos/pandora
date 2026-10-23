//! The read side of the protocol, shared by host and app; each owns what a write does
//! with the vault afterwards.

use crate::bridge::protocol::{Candidate, CodeInfo, Request, Response};
use crate::vault::matching;
use crate::vault::model::{now_secs, Login, Otp, Vault};
use crate::vault::totp;
use zeroize::Zeroizing;

/// The box behind a request; `Locked` is the absence of one, not an error.
#[derive(Clone, Copy)]
pub enum Peer<'a> {
    /// The host can open a box, the app cannot: that is its window's job.
    Locked {
        unlockable: bool,
    },
    Open(&'a Vault),
}

/// Answer a request both peers answer the same way, or `None` for the caller's own.
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

        Request::PasskeyList { rp_id } => Some(match peer {
            Peer::Locked { .. } => locked(),
            Peer::Open(vault) => Response::Passkeys {
                passkeys: crate::host::passkeys::list(vault, rp_id),
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

/// Add a login the panel saved; how the change reaches disk is the peer's own.
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

/// Change the sent fields; anything left out keeps what it had. `None` for an unknown id.
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
        item.set_password(password);
    }
    item.touched(now_secs());
    Some(candidate(
        item,
        (!item.url.is_empty()).then_some(item.url.as_str()),
    ))
}

/// Every entry, for the popup's browsing; never a password.
pub fn entries(vault: &Vault) -> Vec<Candidate> {
    vault
        .logins
        .iter()
        .map(|item| candidate(item, (!item.url.is_empty()).then_some(item.url.as_str())))
        .collect()
}

/// Entries claiming the page, most specific first, each naming the URL that matched.
pub fn matches(vault: &Vault, page: &str) -> Vec<Candidate> {
    rank(&vault.logins, page)
}

/// The same ranking over a set of logins a caller already holds.
pub fn rank(logins: &[Login], page: &str) -> Vec<Candidate> {
    let Some(page) = matching::parse(page) else {
        return Vec::new();
    };
    let mut hits: Vec<(&Login, matching::Rank, &str)> = logins
        .iter()
        .filter_map(|item| matching::rank(item, &page).map(|(rank, url)| (item, rank, url)))
        .collect();
    // More specific first, then by name, so the order never wobbles between calls.
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    hits.into_iter()
        .map(|(item, _, url)| candidate(item, Some(url)))
        .collect()
}

/// The one place a password enters a response, only for the id the user picked.
pub fn reveal(vault: &Vault, id: &str) -> Option<(String, String)> {
    vault
        .logins
        .iter()
        .find(|item| item.id == id)
        .map(|item| (item.username.clone(), item.password.to_string()))
}

/// Every seed by name: the seed never leaves the box, and neither does a code.
pub fn codes(vault: &Vault) -> Vec<CodeInfo> {
    listing(&vault.otps)
}

/// The same listing over seeds a caller already holds.
pub fn listing(otps: &[Otp]) -> Vec<CodeInfo> {
    otps.iter()
        .map(|otp| CodeInfo {
            id: otp.id.clone(),
            name: otp.name.clone(),
        })
        .collect()
}

/// The code a seed holds now, derived on the spot for one id: what a page needs,
/// never the seed itself.
pub fn code(vault: &Vault, id: &str, now: u64) -> Option<(Zeroizing<String>, u64)> {
    derive(vault.otps.iter().find(|otp| otp.id == id)?, now)
}

/// The same derivation over one seed, for a caller that holds no box.
pub fn derive(otp: &Otp, now: u64) -> Option<(Zeroizing<String>, u64)> {
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

/// The one error every locked request gets; the extension turns it into "click to unlock".
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

    /// What goes back is exactly what the seed derives at that second, for that id.
    #[test]
    fn one_code_is_derived_for_one_id_at_a_time() {
        let vault = boxed();
        let (expected, left) = totp::at(SEED, 1_700_000_000).unwrap();
        let (derived, remaining) = code(&vault, &vault.otps[0].id, 1_700_000_000).unwrap();
        assert_eq!(derived, expected);
        assert_eq!(remaining, left);

        // The real clock is used, so only the answer's shape and its one failure are checked.
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

        // An unknown id is an error, not an empty code.
        let answer = read(
            &Request::Code {
                id: "no such seed".to_owned(),
            },
            Peer::Open(&vault),
        )
        .unwrap();
        assert!(matches!(answer, Response::Error { .. }), "{answer:?}");
    }

    /// The listing is the shape a page can use: id, name, algorithm and handle. The
    /// key stays in the box, and another site's credential is not even named.
    #[test]
    fn listing_passkeys_carries_no_key_and_only_its_own_site() {
        let mut vault = Vault::new("Box");
        let mut it = crate::vault::model::Passkey::new(1, "example.com");
        it.name = "ana@example.com".to_owned();
        it.credential_id = Zeroizing::new(vec![1, 2, 3]);
        it.user_handle = Zeroizing::new(vec![7, 7, 7]);
        it.private_key = Zeroizing::new(b"never-listed".to_vec());
        vault.passkeys.push(it);
        let mut other = crate::vault::model::Passkey::new(1, "other.example");
        other.name = "other site".to_owned();
        vault.passkeys.push(other);

        let answer = read(
            &Request::PasskeyList {
                rp_id: "example.com".to_owned(),
            },
            Peer::Open(&vault),
        )
        .unwrap();
        let json = serde_json::to_string(&answer).unwrap();
        assert!(json.contains("ana@example.com"), "{json}");
        assert!(!json.contains("other site"), "another site is not offered");
        assert!(!json.contains("never-listed"), "the key stays in the box");
        match answer {
            Response::Passkeys { passkeys } => {
                assert_eq!(passkeys.len(), 1);
                assert_eq!(passkeys[0].alg, -7, "ES256 is what a host mints");
                assert!(!passkeys[0].user_handle.is_empty());
                assert_eq!(
                    passkeys[0].credential_id,
                    crate::vault::codec::display(&[1, 2, 3]).to_string(),
                    "the site's own credential id, in base64, is what a page names"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// A locked peer answers every read the same way, codes included.
    #[test]
    fn a_locked_peer_carries_no_code() {
        let locked = Peer::Locked { unlockable: false };
        for request in [
            Request::Codes,
            Request::Code { id: "x".to_owned() },
            Request::PasskeyList {
                rp_id: "example.com".to_owned(),
            },
        ] {
            assert!(matches!(
                read(&request, locked).unwrap(),
                Response::Error { .. }
            ));
        }
    }
}
