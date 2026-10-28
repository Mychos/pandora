//! Keeping the GitHub session in the platform's own credential store, so a
//! signed-in run survives a restart. Opt-in: nothing is written anywhere
//! unless the user asks for it. What is kept is the access token, which does not
//! expire, and what an older build left there — a refresh token — is not one.
//! A session reads nothing on its own — the box still needs its master password
//! — but it is a credential, so it goes to the system store and never to a plain
//! file.
//!
//! A desktop has a keyring; a phone has its keystore, reached through the one
//! Java class the platform side owns (`Pandora.keep`/`recall`/`drop`). Either
//! way the token lands encrypted, under a key the app cannot read off the disk.

#[cfg(not(target_os = "android"))]
use keyring::Entry;

use crate::features::sync::model::Sync;

/// How long a credential store that was not up yet is left alone before another ask.
const RETRY: u64 = 60;

/// How many asks a run makes before it gives up on a store that never answers.
const TRIES: u8 = 5;

/// The name the system store shows for what is asking.
#[cfg(not(target_os = "android"))]
const SERVICE: &str = "Pandora";

/// One GitHub session per user account, on this machine.
#[cfg(not(target_os = "android"))]
const ACCOUNT: &str = "github";

#[cfg(not(target_os = "android"))]
fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| {
        crate::shared::i18n::tf("no system credential store: {error}", &[("error", &e)])
    })
}

#[cfg(not(target_os = "android"))]
pub fn load() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) if !token.is_empty() => Ok(Some(token)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(crate::shared::i18n::tf(
            "cannot read the remembered token: {error}",
            &[("error", &e)],
        )),
    }
}

#[cfg(target_os = "android")]
pub fn load() -> Result<Option<String>, String> {
    crate::shared::android::text("recall")
}

#[cfg(not(target_os = "android"))]
pub fn save(token: &str) -> Result<(), String> {
    entry()?.set_password(token).map_err(|e| {
        crate::shared::i18n::tf("cannot remember the token: {error}", &[("error", &e)])
    })
}

#[cfg(target_os = "android")]
pub fn save(token: &str) -> Result<(), String> {
    crate::shared::android::send("keep", token)
}

impl Sync {
    /// Ask the store for a remembered session. A store that is locked or not up
    /// yet is asked again from the frame loop, not written off for the whole run.
    pub fn recall(&mut self) -> Option<String> {
        self.recall_with(load)
    }

    fn recall_with(
        &mut self,
        load: impl FnOnce() -> Result<Option<String>, String>,
    ) -> Option<String> {
        match load() {
            Ok(Some(kept)) => {
                log::info!("pandora: the store handed back a remembered session");
                self.restore(kept);
                self.recall_at = None;
                self.recalls = 0;
                None
            }
            Ok(None) => {
                log::warn!("pandora: the store holds no remembered session");
                self.recall_at = None;
                self.recalls = 0;
                None
            }
            Err(e) => {
                self.recalls = self.recalls.saturating_add(1);
                self.recall_at =
                    (self.recalls < TRIES).then(|| crate::vault::model::now_secs() + RETRY);
                Some(e)
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn forget() -> Result<(), String> {
    let Ok(entry) = entry() else {
        return Ok(());
    };
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(crate::shared::i18n::tf(
            "cannot forget the token: {error}",
            &[("error", &e)],
        )),
    }
}

#[cfg(target_os = "android")]
pub fn forget() -> Result<(), String> {
    crate::shared::android::send_none("drop")
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    /// A store that was locked at boot is asked again, not written off: the
    /// remembered session lands on the retry and the wait is cleared.
    #[test]
    fn a_store_that_failed_once_is_asked_again() {
        let mut sync = Sync::default();
        let now = crate::vault::model::now_secs();

        let why = sync
            .recall_with(|| Err("the keyring is locked".to_owned()))
            .unwrap();
        assert_eq!(why, "the keyring is locked");
        assert_eq!(sync.recalls, 1);
        assert!(
            sync.recall_at.is_some_and(|at| at > now),
            "the next ask waits instead of one per frame"
        );
        assert!(!sync.signed(), "nothing was taken this time");

        assert!(
            sync.recall_with(|| Ok(Some("gho_kept".to_owned())))
                .is_none(),
            "the retry took the session and has nothing to report"
        );
        assert_eq!(sync.token.as_str(), "gho_kept");
        assert!(sync.recall_at.is_none(), "the wait is over");
        assert_eq!(sync.recalls, 0, "a later boot starts counting again");
    }

    #[test]
    #[ignore = "writes to the platform credential store"]
    fn the_token_round_trips_through_the_store() {
        let entry = Entry::new(SERVICE, "github-round-trip-test").unwrap();
        entry.set_password("gho_secret").unwrap();
        assert_eq!(entry.get_password().unwrap(), "gho_secret");
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
