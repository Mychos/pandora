//! Android's side of a seal: `android/app/src/main/java/dev/pandora/Pandora.kt`
//! owns the keystore key and the prompt, and this is only the base64 in between.
//! Both directions ask for a finger — the key is symmetric, so the keystore will
//! not let it wrap the box's key without one — which is why either call can take
//! a while and why both run on a worker, never on the frame.

use crate::shared::android;
use crate::vault::crypto::{Key, KEY_LEN};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use zeroize::Zeroizing;

pub fn available() -> bool {
    android::flag("available")
}

pub fn seal(key: &Key) -> Result<String, String> {
    let plain = Zeroizing::new(STANDARD.encode(&key[..]));
    android::text_with("seal", &plain)?.ok_or_else(|| super::CANCELED.to_owned())
}

pub fn unseal(blob: &str) -> Result<Key, String> {
    let plain = android::text_with("unseal", blob)?.ok_or_else(|| super::CANCELED.to_owned())?;
    let plain = Zeroizing::new(plain);
    let bytes = Zeroizing::new(STANDARD.decode(plain.as_bytes()).map_err(|e| {
        crate::shared::i18n::tf("the keystore returned junk: {error}", &[("error", &e)])
    })?);
    let key: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
        crate::shared::i18n::t("the keystore returned the wrong key length").to_owned()
    })?;
    Ok(Zeroizing::new(key))
}
