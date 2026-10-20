//! Every platform without a reader the app can ask — the desktop. The door is the
//! master password and nothing else, so the feature is never offered (see
//! `available`), and these calls are the ones the UI would make if it were.

use crate::vault::crypto::Key;

pub fn available() -> bool {
    false
}

pub fn seal(_key: &Key) -> Result<String, String> {
    Err(crate::shared::i18n::t("this platform has no fingerprint reader").to_owned())
}

pub fn unseal(_blob: &str) -> Result<Key, String> {
    Err(crate::shared::i18n::t("this platform has no fingerprint reader").to_owned())
}
