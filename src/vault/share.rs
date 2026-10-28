//! A record sealed for somebody else: a `.pandora-share` file that only one recipient
//! can open, with no account and no server in between.
//!
//! The shape is age's, cut down to what this app needs. The file is a magic line, a
//! JSON header and one XChaCha20-Poly1305 ciphertext. The header rides in the AAD, so
//! the recipient, the salt and the moment cannot be rewritten under the ciphertext.
//!
//! Two ways to be the recipient:
//! - **A key.** The sender seals to the recipient's X25519 public key; the shared
//!   secret is the exchange between an ephemeral key made for this file and theirs.
//!   The keypair lives inside the box payload, so it is protected like everything else.
//! - **A passphrase.** Argon2id over the passphrase and a random salt, with the same
//!   parameters the box uses. Nothing has to be exchanged in advance; the passphrase
//!   is, which is why it is the weaker of the two and said so where it is asked for.

use crate::shared::utils::import::Bundle;
use crate::vault::codec;
use crate::vault::crypto::{self, KdfParams, Key};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub const EXT: &str = "pandora-share";
pub const MAGIC: &[u8] = b"PANDORASHARE1\n";

/// The keypair a box keeps for receiving shares. Its private half is a secret like any
/// other: base64 in the payload, `Zeroizing` in memory, never in a `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keypair {
    #[serde(with = "secret_bytes")]
    pub private: Zeroizing<Vec<u8>>,
    #[serde(with = "plain_bytes")]
    pub public: Vec<u8>,
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keypair")
            .field("private", &"[REDACTED]")
            .field("public", &codec::display(&self.public).to_string())
            .finish()
    }
}

impl Keypair {
    /// A fresh keypair, made the first time a box seals or receives by key.
    pub fn make() -> Self {
        let private = Zeroizing::new(crypto::random_bytes::<32>().to_vec());
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&private);
        let public = PublicKey::from(&StaticSecret::from(seed)).to_bytes();
        Self {
            private,
            public: public.to_vec(),
        }
    }

    /// The line a recipient hands to a sender.
    pub fn public_text(&self) -> String {
        public_text(&self.public)
    }
}

/// Bytes ride as base64, never as an array of numbers: the box's own convention.
mod secret_bytes {
    use crate::vault::codec;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(bytes: &Zeroizing<Vec<u8>>, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(&codec::display(bytes.as_ref()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Zeroizing<Vec<u8>>, D::Error> {
        let text = String::deserialize(de)?;
        let bytes = codec::decode(&text).map_err(serde::de::Error::custom)?;
        Ok(Zeroizing::new(bytes))
    }
}

mod plain_bytes {
    use crate::vault::codec;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(&codec::display(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(de)?;
        codec::decode(&text).map_err(serde::de::Error::custom)
    }
}

/// The public key as that line: base64, which is what a chat window keeps intact.
pub fn public_text(public: &[u8]) -> String {
    codec::display(public).to_string()
}

/// Read back what a recipient pasted, spaces and dashes and all.
pub fn parse_public(text: &str) -> Result<[u8; 32], String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let bytes = codec::decode(&cleaned)
        .map_err(|_| crate::shared::i18n::t("that is not a share key").to_owned())?;
    to_array(&bytes)
}

/// A public key as the 32 bytes the exchange takes, from the same line a recipient
/// hands out.
pub fn public_array(text: &str) -> Result<[u8; 32], String> {
    parse_public(text)
}

/// Who the file is for, and how it is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Sealed to a key: the recipient's public key opens it.
    Key,
    /// Sealed to a passphrase, which has to be told apart from the file.
    Passphrase,
}

/// The header, in the clear because it says nothing secret: who, how, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Header {
    mode: String,
    nonce: String,
    created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ephemeral: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipient: Option<String>,
    /// The box the records come from; a name, never a path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    from: String,
}

impl Header {
    fn mode(&self) -> Mode {
        if self.mode == "key" {
            Mode::Key
        } else {
            Mode::Passphrase
        }
    }
}

/// What a file says about itself before anything is decrypted: how it opens, who sealed
/// it and when. `None` when it is not a share at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub mode: Mode,
    pub from: String,
    pub created: u64,
}

pub fn info(bytes: &[u8]) -> Option<Info> {
    let (header, _) = split(bytes)?;
    Some(Info {
        mode: header.mode(),
        from: header.from,
        created: header.created,
    })
}

/// Seal a bundle for one recipient. `now` is passed in so the file can be tested.
pub fn seal(bundle: &Bundle, from: &str, mode: &SealTo<'_>, now: u64) -> Result<Vec<u8>, String> {
    let payload = Zeroizing::new(
        serde_json::to_vec(bundle).map_err(|e| format!("cannot write the share: {e}"))?,
    );
    let (header, key) = match mode {
        SealTo::Key(public) => {
            let ephemeral = Zeroizing::new(crypto::random_bytes::<32>().to_vec());
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&ephemeral);
            let secret = StaticSecret::from(seed);
            let mine = PublicKey::from(&secret);
            let shared = secret.diffie_hellman(&PublicKey::from(*public));
            let key = exchange(shared.as_bytes(), mine.as_bytes(), public.as_slice());
            eprintln!(
                "SEAL shared={:02x?} mine={:02x?} pub={:02x?}",
                shared.as_bytes(),
                mine.as_bytes(),
                public
            );
            let header = Header {
                mode: "key".to_owned(),
                nonce: String::new(),
                created: now,
                salt: None,
                // The public half only: the ephemeral secret never leaves this function.
                ephemeral: Some(codec::display(mine.as_bytes()).to_string()),
                recipient: Some(public_text(public.as_slice())),
                from: from.to_owned(),
            };
            (header, key)
        }
        SealTo::Passphrase(passphrase) => {
            let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
            let key = crypto::derive_key(passphrase.as_bytes(), &salt, &KdfParams::default())?;
            let header = Header {
                mode: "passphrase".to_owned(),
                nonce: String::new(),
                created: now,
                salt: Some(codec::display(&salt).to_string()),
                ephemeral: None,
                recipient: None,
                from: from.to_owned(),
            };
            (header, key)
        }
    };

    // The header is the AAD, so its own fields are what the tag authenticates; the
    // nonce is filled in after it is encrypted, which is why it is not part of them.
    let aad = serde_json::to_vec(&header).map_err(|e| format!("cannot write the share: {e}"))?;
    let mut body = payload.to_vec();
    let nonce = crypto::encrypt_in_place(&mut body, &key, &aad)?;

    let mut header = header;
    header.nonce = codec::display(&nonce).to_string();
    let mut out = Vec::with_capacity(MAGIC.len() + aad.len() + body.len() + 128);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(
        &serde_json::to_vec(&header).map_err(|e| format!("cannot write the share: {e}"))?,
    );
    out.push(b'\n');
    out.extend_from_slice(&body);
    Ok(out)
}

/// Open a share with one of the two keys a recipient can hold.
pub fn open(
    bytes: &[u8],
    identity: Option<&Keypair>,
    passphrase: Option<&str>,
) -> Result<Bundle, String> {
    let (header, body) = split(bytes)
        .ok_or_else(|| crate::shared::i18n::t("that is not a Pandora share").to_owned())?;
    let key = match header.mode() {
        Mode::Key => {
            let identity = identity.ok_or_else(|| {
                crate::shared::i18n::t("this share is sealed to a key").to_owned()
            })?;
            let ephemeral = codec::decode(header.ephemeral.as_deref().unwrap_or_default())
                .map_err(|_| crate::shared::i18n::t("that share is damaged").to_owned())?;
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&identity.private);
            let secret = StaticSecret::from(seed);
            let shared = secret.diffie_hellman(&PublicKey::from(to_array(&ephemeral)?));
            eprintln!(
                "OPEN shared={:02x?} eph={:02x?} pub={:02x?}",
                shared.as_bytes(),
                ephemeral,
                identity.public
            );
            exchange(shared.as_bytes(), &ephemeral, &identity.public)
        }
        Mode::Passphrase => {
            let passphrase = passphrase.ok_or_else(|| {
                crate::shared::i18n::t("this share asks for a passphrase").to_owned()
            })?;
            let salt = codec::decode(header.salt.as_deref().unwrap_or_default())
                .map_err(|_| crate::shared::i18n::t("that share is damaged").to_owned())?;
            crypto::derive_key(passphrase.as_bytes(), &salt, &KdfParams::default())?
        }
    };
    let nonce = codec::decode(&header.nonce)
        .map_err(|_| crate::shared::i18n::t("that share is damaged").to_owned())?;
    let mut aad = header.clone();
    aad.nonce = String::new();
    let aad = serde_json::to_vec(&aad)
        .map_err(|_| crate::shared::i18n::t("that share is damaged").to_owned())?;
    let payload = crypto::decrypt(&nonce, body, &key, &aad)
        .map_err(|_| crate::shared::i18n::t("this share does not open with that key").to_owned())?;
    serde_json::from_slice(&payload)
        .map_err(|_| crate::shared::i18n::t("that share is damaged").to_owned())
}

/// What the sender sealed for, so the recipient knows which key to reach for.
pub enum SealTo<'a> {
    Key([u8; 32]),
    Passphrase(&'a str),
}

fn split(bytes: &[u8]) -> Option<(Header, &[u8])> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let end = rest.iter().position(|b| *b == b'\n')?;
    let header: Header = serde_json::from_slice(&rest[..end]).ok()?;
    Some((header, &rest[end + 1..]))
}

/// The X25519 exchange, bound to both public keys so a swapped ephemeral key is a
/// different secret and the tag refuses it.
fn exchange(shared: &[u8; 32], ephemeral: &[u8], recipient: &[u8]) -> Key {
    let mut hasher = Sha256::new();
    hasher.update(shared);
    hasher.update(ephemeral);
    hasher.update(recipient);
    hasher.update(b"pandora-share");
    Zeroizing::new(hasher.finalize().into())
}

fn to_array(bytes: &[u8]) -> Result<[u8; 32], String> {
    bytes
        .try_into()
        .map_err(|_| crate::shared::i18n::t("that share key is the wrong length").to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{now_secs, Login, Otp};
    use zeroize::Zeroizing as Z;

    fn bundle() -> Bundle {
        let mut login = Login::new(1);
        login.name = "Bank".to_owned();
        login.password = Z::new("hunter2".to_owned());
        let mut otp = Otp::new(1);
        otp.name = "Mail".to_owned();
        otp.secret = Z::new("JBSWY3DPEHPK3PXP".to_owned());
        Bundle {
            logins: vec![login],
            otps: vec![otp],
            ..Default::default()
        }
    }

    /// A key share opens only with the key it was sealed to, and the records come back
    /// as the shapes they were.
    #[test]
    fn a_key_share_opens_only_with_its_own_key() {
        let alice = Keypair::make();
        let bob = Keypair::make();
        let to = public_array(&alice.public_text()).unwrap();
        let bytes = seal(&bundle(), "Ana's box", &SealTo::Key(to), 42).unwrap();

        let info = info(&bytes).unwrap();
        assert_eq!(info.mode, Mode::Key);
        assert_eq!(info.from, "Ana's box");
        assert_eq!(info.created, 42);

        let opened = open(&bytes, Some(&alice), None).unwrap();
        assert_eq!(opened.logins[0].password.as_str(), "hunter2");
        assert_eq!(opened.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");

        assert!(
            open(&bytes, Some(&bob), None).is_err(),
            "another key does not open it"
        );
        assert!(
            open(&bytes, None, Some("anything")).is_err(),
            "nor a passphrase"
        );
    }

    /// A passphrase share: the same one opens it, the wrong one does not, and asking a
    /// key holder for it changes nothing.
    #[test]
    fn a_passphrase_share_opens_only_with_its_passphrase() {
        let bytes = seal(&bundle(), "Box", &SealTo::Passphrase("correct horse"), 1).unwrap();
        assert_eq!(info(&bytes).unwrap().mode, Mode::Passphrase);
        let opened = open(&bytes, None, Some("correct horse")).unwrap();
        assert_eq!(opened.logins[0].name, "Bank");
        assert!(open(&bytes, None, Some("wrong horse")).is_err());
        assert!(open(&bytes, Some(&Keypair::make()), None).is_err());
    }

    /// The header is authenticated: changing who the file is for, or when it was made,
    /// is a file that will not open.
    #[test]
    fn the_header_cannot_be_rewritten_under_the_ciphertext() {
        let key = Keypair::make();
        let public: [u8; 32] = key.public.as_slice().try_into().unwrap();
        let bytes = seal(&bundle(), "Box", &SealTo::Key(public), 7).unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let tampered = text.replace("\"created\":7", "\"created\":8");
        assert_ne!(tampered, text);
        assert!(
            open(tampered.as_bytes(), Some(&key), None).is_err(),
            "the tag covers the header"
        );

        // And a flipped byte in the body is a share that does not open either.
        let mut body = bytes.clone();
        let last = body.len() - 1;
        body[last] ^= 0xFF;
        assert!(open(&body, Some(&key), None).is_err());
    }

    /// What a recipient pastes is read back the way it was printed, and a stranger is
    /// refused before a file is made.
    #[test]
    fn a_public_key_is_a_line_a_chat_can_carry() {
        let key = Keypair::make();
        let line = key.public_text();
        assert_eq!(parse_public(&line).unwrap(), key.public.as_slice());
        assert_eq!(
            parse_public(&format!(" {} \n", line)).unwrap(),
            key.public.as_slice(),
            "spaces and newlines are not the key"
        );
        assert!(parse_public("not a key").is_err());
        assert!(
            parse_public(&codec::display(&[0u8; 5]).to_string()).is_err(),
            "short"
        );
        assert!(!format!("{key:?}").contains("private-key"), "the key hides");
    }

    #[test]
    fn a_file_that_is_not_a_share_is_refused() {
        assert!(info(b"not a share").is_none());
        assert!(open(b"PANDORASHARE1\n{}\nbody", None, Some("x")).is_err());
        let bytes = seal(&bundle(), "Box", &SealTo::Passphrase("pw"), now_secs()).unwrap();
        assert!(open(&bytes[..bytes.len() - 1], None, Some("pw")).is_err());
    }
}
