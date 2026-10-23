//! The recovery code: a printed line that opens the box when the master password is
//! gone. It is not a second password — the box stays sealed with its own key — it is a
//! copy of that key wrapped under a code the user writes down.
//!
//! The wrapped key lives in the box's header, in the clear like the KDF parameters and
//! for the same reason: it has to be readable before anything is decrypted. It says
//! nothing on its own — the code is the only thing that opens it — and it is inside the
//! header's AAD, so it cannot be swapped under the ciphertext.

use crate::vault::codec;
use crate::vault::crypto::{self, KdfParams, Key};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// How many bytes a code carries. A person copies this once, so it is long enough that
/// guessing it is not a plan: 256 bits.
const CODE_LEN: usize = 32;

/// The wrapped key, in the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// base64
    pub salt: String,
    /// base64
    pub nonce: String,
    /// base64 of the box key, encrypted under the code
    pub wrapped: String,
}

impl Slot {
    /// Whether a code looks like one of ours before any derivation runs: base32, and
    /// the right number of bytes. Cheap, so a typo costs no Argon2.
    pub fn reads(code: &str) -> bool {
        decode_code(code).is_ok()
    }
}

/// A fresh code, grouped in fours so it can be read aloud and written down: 256 bits of
/// base32 is 52 characters.
pub fn make_code() -> Zeroizing<String> {
    let bytes = crypto::random_bytes::<CODE_LEN>();
    let text = BASE32_NOPAD.encode(&bytes);
    Zeroizing::new(
        text.as_bytes()
            .chunks(4)
            .map(|it| std::str::from_utf8(it).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// What a user typed, as the bytes the derivation takes: letters are case-folded and
/// anything that is not base32 (spaces, dashes, a stray `0`) is dropped before checking
/// the length.
fn decode_code(code: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let bytes = BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .map_err(|_| crate::shared::i18n::t("that is not a recovery code").to_owned())?;
    if bytes.len() != CODE_LEN {
        return Err(crate::shared::i18n::t("that is not a recovery code").to_owned());
    }
    Ok(bytes)
}

fn aad(box_id: &str, salt: &str) -> Vec<u8> {
    format!("pandora-recovery|{box_id}|{salt}").into_bytes()
}

/// Wrap the box's key under a code. The key itself never changes: this is a copy.
pub fn wrap(key: &Key, code: &str, box_id: &str) -> Result<Slot, String> {
    let secret = Zeroizing::new(decode_code(code)?);
    let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
    let salt_text = codec::display(&salt).to_string();
    let wrapping = crypto::derive_key(secret.as_slice(), &salt, &KdfParams::default())?;
    let mut plain = Zeroizing::new(key.to_vec());
    let nonce = crypto::encrypt_in_place(&mut plain, &wrapping, &aad(box_id, &salt_text))?;
    let nonce = codec::display(&nonce).to_string();
    let wrapped = codec::display(plain.as_slice()).to_string();
    Ok(Slot {
        salt: salt_text,
        nonce,
        wrapped,
    })
}

/// Open the copy with the code. A wrong code is a wrong key, and the tag says so.
pub fn unwrap(slot: &Slot, code: &str, box_id: &str) -> Result<Key, String> {
    let secret = Zeroizing::new(decode_code(code)?);
    let salt = codec::decode(&slot.salt)?;
    let nonce = codec::decode(&slot.nonce)?;
    let wrapped = codec::decode(&slot.wrapped)?;
    let wrapping = crypto::derive_key(secret.as_slice(), &salt, &KdfParams::default())?;
    let plain = crypto::decrypt(&nonce, &wrapped, &wrapping, &aad(box_id, &slot.salt))
        .map_err(|_| crate::shared::i18n::t("that code does not open this box").to_owned())?;
    let key: [u8; crypto::KEY_LEN] = plain
        .as_slice()
        .try_into()
        .map_err(|_| crate::shared::i18n::t("that code does not open this box").to_owned())?;
    Ok(Zeroizing::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::crypto::{derive_key, random_bytes};

    fn key() -> Key {
        let salt = random_bytes::<{ crypto::SALT_LEN }>();
        derive_key(b"a password", &salt, &KdfParams::default()).unwrap()
    }

    /// The code opens a copy of the key, and only the right code does — letters case
    /// and the printed dashes are not part of it.
    #[test]
    fn a_code_opens_a_copy_of_the_key() {
        let key = key();
        let code = make_code();
        let slot = wrap(&key, &code, "box-id").unwrap();

        let opened = unwrap(&slot, &code, "box-id").unwrap();
        assert_eq!(opened.as_ref(), key.as_ref());

        let typed = format!("  {}  ", code.to_lowercase().replace('-', " "));
        assert_eq!(
            unwrap(&slot, &typed, "box-id").unwrap().as_ref(),
            key.as_ref()
        );

        assert!(unwrap(&slot, &format!("{}AAAA", code.as_str()), "box-id").is_err());
        assert!(unwrap(&slot, &code, "another-box").is_err());
        assert!(unwrap(&slot, "not a code", "box-id").is_err());
    }

    /// A slot is a salt, a nonce and 32 bytes: nothing in it says what the key is, and
    /// two wraps of one key never look alike.
    #[test]
    fn a_slot_says_nothing_and_never_repeats() {
        let key = key();
        let code = make_code();
        let first = wrap(&key, &code, "box-id").unwrap();
        let second = wrap(&key, &code, "box-id").unwrap();
        assert_ne!(first.salt, second.salt);
        assert_ne!(first.wrapped, second.wrapped);
        assert_eq!(
            codec::decode(&first.wrapped).unwrap().len(),
            crypto::KEY_LEN + 16
        );
        assert!(!format!("{first:?}").contains(&code[..8]));
    }

    #[test]
    fn a_code_is_256_bits_grouped_in_fours() {
        let code = make_code();
        assert_eq!(
            code.chars().filter(|c| *c != '-').count(),
            52,
            "256 bits of base32"
        );
        let groups: Vec<&str> = code.split('-').collect();
        assert!(
            groups[..groups.len() - 1]
                .iter()
                .all(|group| group.len() == 4),
            "every group but the last is four characters"
        );
        assert!(Slot::reads(&code));
        assert!(!Slot::reads("AAAA"));
        assert_ne!(
            make_code().as_str(),
            make_code().as_str(),
            "two codes differ"
        );
    }
}
