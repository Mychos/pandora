//! The one optional lookup in the review: is this password in a public breach?
//!
//! It is off until the user turns it on, and what leaves is a five-character SHA-1
//! prefix — the half of the hash the answer cannot be turned back from, which is
//! how Have I Been Pwned's range API works. The suffix stays here and is compared
//! locally, so a request says "somebody, somewhere, has one of these 800 passwords"
//! and never which. No user, no address, no box.

use crate::vault::model::Vault;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use zeroize::Zeroizing;

#[cfg(not(test))]
const API: &str = "https://api.pwnedpasswords.com/range/";
#[cfg(not(test))]
const TIMEOUT: u64 = 20;
/// The most prefixes one run may ask about: a box of thousands is not a reason to
/// spend an afternoon on somebody else's API.
const MAX_PREFIXES: usize = 40;

#[derive(Default)]
pub struct Check {
    job: Option<Job>,
    /// Passwords found in a breach, and how many times: `(record id, count)`.
    pub found: Vec<(String, u64)>,
    /// How many passwords were asked about, and how many prefixes that took.
    pub asked: usize,
    pub prefixes: usize,
    pub error: Option<String>,
    /// Whether a run was ever asked for this box; without it a start that answered
    /// nothing would be asked again every frame.
    started: bool,
    /// Which box the answers belong to; another box is asked again.
    key: Option<String>,
}

struct Job {
    rx: Receiver<Reply>,
}

// The suite never starts a worker, so what it answers is only built outside it.
#[cfg_attr(test, allow(dead_code))]
enum Reply {
    Done {
        found: Vec<(String, u64)>,
        asked: usize,
        prefixes: usize,
    },
    Failed(String),
}

impl Check {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// The answers this box already has; `false` asks for a run.
    pub fn covers(&self, key: &str) -> bool {
        self.key.as_deref() == Some(key) && self.started
    }

    /// Forget everything: the switch was turned off, or another box opened.
    pub fn clear(&mut self) {
        self.job = None;
        self.found.clear();
        self.asked = 0;
        self.prefixes = 0;
        self.error = None;
        self.started = false;
        self.key = None;
    }

    /// Ask about the box's passwords, in the background. A lookup already running is
    /// not started twice.
    pub fn start(&mut self, key: &str, vault: &Vault) {
        if self.running() {
            return;
        }
        let passwords: Vec<(String, String)> = vault
            .logins
            .iter()
            .filter(|it| !it.password.is_empty())
            .map(|it| (it.id.clone(), it.password.to_string()))
            .collect();
        self.key = Some(key.to_owned());
        self.started = true;
        self.found.clear();
        self.error = None;
        self.asked = 0;
        self.prefixes = 0;

        // Compiled out of a test build, like the update check: the suite drives `scan`
        // with a fetcher of its own and never reaches the network.
        #[cfg(not(test))]
        {
            let (tx, rx) = mpsc::channel();
            self.job = Some(Job { rx });
            std::thread::spawn(move || {
                let reply = match scan(passwords, fetch) {
                    Ok((found, asked, prefixes)) => Reply::Done {
                        found,
                        asked,
                        prefixes,
                    },
                    Err(e) => Reply::Failed(e),
                };
                let _ = tx.send(reply);
            });
        }
        #[cfg(test)]
        let _ = passwords;
    }

    /// Land a finished worker and keep frames coming while one is out.
    pub fn pump(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Reply::Done {
                found,
                asked,
                prefixes,
            }) => {
                self.found = found;
                self.asked = asked;
                self.prefixes = prefixes;
                self.job = None;
            }
            Ok(Reply::Failed(e)) => {
                self.error = Some(e);
                self.job = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(120)),
            Err(TryRecvError::Disconnected) => {
                self.error = Some("the check was interrupted".to_owned());
                self.job = None;
            }
        }
    }
}

/// One password's SHA-1 in hex, which is the only thing a request is made of.
pub fn hash(password: &str) -> Zeroizing<String> {
    let digest = Sha1::digest(password.as_bytes());
    Zeroizing::new(
        digest
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>(),
    )
}

/// How many times a hash's suffix appears in a range answer; the padding the API
/// sends back is a zero, which is the same as absent.
pub fn count(body: &str, suffix: &str) -> u64 {
    for line in body.lines() {
        let Some((said, times)) = line.split_once(':') else {
            continue;
        };
        if said.trim().eq_ignore_ascii_case(suffix) {
            return times.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Ask about every password, one request per distinct prefix. `fetch` is a prefix
/// and the range answer — a parameter so a test can answer without a network.
/// What a run found, how many passwords it asked about and how many requests that took.
pub type Found = (Vec<(String, u64)>, usize, usize);

pub fn scan(
    passwords: Vec<(String, String)>,
    mut fetch: impl FnMut(&str) -> Result<String, String>,
) -> Result<Found, String> {
    let mut by_prefix: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (id, password) in passwords {
        let hash = hash(&password);
        let (prefix, suffix) = hash.split_at(5);
        by_prefix
            .entry(prefix.to_owned())
            .or_default()
            .push((id, suffix.to_owned()));
    }
    let asked = by_prefix.values().map(Vec::len).sum();
    let mut prefixes = 0;
    let mut found = Vec::new();
    for (prefix, items) in by_prefix {
        if prefixes == MAX_PREFIXES {
            break;
        }
        let body = fetch(&prefix)?;
        prefixes += 1;
        for (id, suffix) in items {
            let times = count(&body, &suffix);
            if times > 0 {
                found.push((id, times));
            }
        }
    }
    Ok((found, asked, prefixes))
}

/// One range request: the prefix only, and `Add-Padding` so a known and an unknown
/// hash come back the same size.
#[cfg(not(test))]
fn fetch(prefix: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(TIMEOUT)))
        .http_status_as_error(false)
        .user_agent(concat!("pandora/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut res = agent
        .get(format!("{API}{prefix}"))
        .header("Add-Padding", "true")
        .call()
        .map_err(|e| format!("haveibeenpwned: {e}"))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err(format!("haveibeenpwned: HTTP {status}"));
    }
    res.body_mut()
        .read_to_string()
        .map_err(|e| format!("haveibeenpwned: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-1 of "password", which every range answer knows.
    const PASSWORD: &str = "password";
    const PREFIXED: &str = "5BAA6";

    #[test]
    fn only_the_prefix_is_the_question() {
        let one = hash(PASSWORD);
        assert_eq!(one.as_str(), "5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8");
        assert_eq!(&one[..5], PREFIXED);
        assert_ne!(hash("password"), hash("password2"));
    }

    #[test]
    fn an_answer_is_read_by_suffix_and_ignores_case_and_padding() {
        let body = "1E4C9B93F3F0682250B6CF8331B7EE68FD8:9545824\r\n\
                    000000000000000000000000000000000A:0\r\n\
                    003D68EB55068C33ACE09247EE4C639306B:3\r\n";
        let hash = hash(PASSWORD);
        assert_eq!(count(body, &hash[5..]), 9545824);
        assert_eq!(count(body, "003D68EB55068C33ACE09247EE4C639306B"), 3);
        assert_eq!(count(body, "0000000000000000000000000000000000"), 0);
        assert_eq!(count("", &hash[5..]), 0);
        assert_eq!(count("not a range answer", &hash[5..]), 0);
    }

    /// Two passwords under one prefix cost one request, and the suffix decides who
    /// was found: the network never sees it.
    #[test]
    fn one_request_per_prefix_and_the_suffix_decides() {
        let mut asked: Vec<String> = Vec::new();
        let hash = hash(PASSWORD);
        let answer = format!("{}:77\n", &hash[5..]).to_owned();
        let (found, checked, prefixes) = scan(
            vec![
                ("a".to_owned(), PASSWORD.to_owned()),
                ("b".to_owned(), "not in any breach".to_owned()),
            ],
            |prefix| {
                asked.push(prefix.to_owned());
                Ok(if prefix == PREFIXED {
                    answer.clone()
                } else {
                    String::new()
                })
            },
        )
        .unwrap();
        assert_eq!(checked, 2);
        assert_eq!(prefixes, 2);
        assert_eq!(asked.len(), 2, "one request each, never one per password");
        assert_eq!(
            found,
            [("a".to_owned(), 77)],
            "only the found one comes back"
        );
    }

    #[test]
    fn a_failure_stops_the_run_instead_of_half_answering() {
        let err = scan(vec![("a".to_owned(), PASSWORD.to_owned())], |_| {
            Err("offline".to_owned())
        })
        .unwrap_err();
        assert_eq!(err, "offline");
    }
}
