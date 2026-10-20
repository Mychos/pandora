//! Keeping the GitHub session in the platform's own credential store, so a
//! signed-in run survives a restart. Opt-in: nothing is written anywhere
//! unless the user asks for it. What is kept is the refresh token when there is
//! one — the access token would be dead within the hour anyway — and the access
//! token itself when the app issues no expiring ones. A session reads nothing on
//! its own — the box still needs its master password — but it is a credential,
//! so it goes to the system store and never to a plain file.
//!
//! A desktop has a keyring; a phone has its keystore, reached through the one
//! Java class the platform side owns (`Pandora.keep`/`recall`/`drop`). Either
//! way the token lands encrypted, under a key the app cannot read off the disk.

#[cfg(not(target_os = "android"))]
use keyring::Entry;

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
