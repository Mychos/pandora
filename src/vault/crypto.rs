//! Key derivation, authenticated encryption and the master-password policy; no egui.

use crate::vault::Failure;
use argon2::{Algorithm, Argon2, Params, Version};
use blake2::{Blake2b512, Digest};
use chacha20poly1305::{
    aead::{Aead, AeadInOut, KeyInit, Payload},
    Key as ChaChaKey, XChaCha20Poly1305, XNonce,
};
use zeroize::Zeroizing;

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;
pub const KEY_LEN: usize = 32;

/// Argon2id params — Bitwarden-grade (64 MiB, 3 passes, 4 lanes).
pub const M_COST: u32 = 64 * 1024;
pub const T_COST: u32 = 3;
pub const P_COST: u32 = 4;

pub type Key = Zeroizing<[u8; KEY_LEN]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost: M_COST,
            t_cost: T_COST,
            p_cost: P_COST,
        }
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS random source unavailable");
    b
}

/// The password plus a BLAKE2b digest of the keyfile: the file alone is never a key.
pub fn material(password: &str, keyfile: Option<&[u8]>) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(password.as_bytes().to_vec());
    if let Some(k) = keyfile {
        out.extend_from_slice(&Blake2b512::digest(k));
    }
    out
}

pub fn derive_key(secret: &[u8], salt: &[u8], params: &KdfParams) -> Result<Key, String> {
    let p = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_LEN))
        .map_err(|e| format!("kdf params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(secret, salt, &mut key)
        .map_err(|e| format!("kdf failed: {e}"))?;
    Ok(Zeroizing::new(key))
}

/// 128 bits of BLAKE2b in hex: a label for something whose real name must not be
/// written down, not a key.
pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finish()
}

/// The same digest, fed a piece at a time: a file is named without holding it all.
#[derive(Default)]
pub struct Hasher(Blake2b512);

impl Hasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> String {
        let full = self.0.finalize();
        full[..16].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// `aad` is authenticated, not encrypted: the vault binds its header and chunks this way.
pub fn encrypt(plain: &[u8], key: &Key, aad: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), String> {
    let k: &[u8] = key.as_ref();
    let key = ChaChaKey::try_from(k).map_err(|_| String::from("bad key length"))?;
    let cipher = XChaCha20Poly1305::new(&key);
    let nonce = random_bytes::<NONCE_LEN>();
    let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| String::from("bad nonce"))?;
    let ct = cipher
        .encrypt(&nonce, Payload { msg: plain, aad })
        .map_err(|e| format!("encrypt failed: {e}"))?;
    Ok((nonce.into(), ct))
}

/// Encrypt in place, appending the tag: a save never holds a second copy of the payload.
pub fn encrypt_in_place(
    plain: &mut Vec<u8>,
    key: &Key,
    aad: &[u8],
) -> Result<[u8; NONCE_LEN], String> {
    let k: &[u8] = key.as_ref();
    let key = ChaChaKey::try_from(k).map_err(|_| String::from("bad key length"))?;
    let cipher = XChaCha20Poly1305::new(&key);
    let nonce = random_bytes::<NONCE_LEN>();
    let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| String::from("bad nonce"))?;
    // Reserve the tag so the buffer does not regrow and copy the payload at the end.
    plain.reserve(16);
    cipher
        .encrypt_in_place(&nonce, aad, plain)
        .map_err(|e| format!("encrypt failed: {e}"))?;
    Ok(nonce.into())
}

/// The AEAD tag is the whole check: a failure is the same one a wrong password gives.
pub fn decrypt(nonce: &[u8], ciphertext: &[u8], key: &Key, aad: &[u8]) -> Result<Vec<u8>, String> {
    let k: &[u8] = key.as_ref();
    let key = ChaChaKey::try_from(k).map_err(|_| String::from("bad key length"))?;
    let cipher = XChaCha20Poly1305::new(&key);
    let nonce = XNonce::try_from(nonce).map_err(|_| String::from("bad nonce"))?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| Failure::Wrong.to_string())
}

pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strength {
    Weak,
    Fair,
    Strong,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Strength::Weak => "weak — longer is better",
            Strength::Fair => "fair — add length or another kind of character",
            Strength::Strong => "strong",
        }
    }
}

const COMMON: [&str; 20] = [
    "123456789012",
    "1234567890123",
    "password1234",
    "password12345",
    "qwertyuiop123",
    "qwerty123456",
    "abc123456789",
    "iloveyou1234",
    "administrator",
    "1qaz2wsx3edc",
    "letmeinplease",
    "football1234",
    "superman1234",
    "sunshine1234",
    "trustno1trustno1",
    "correcthorsebatterystaple",
    "aaaaaaaaaaaa",
    "111111111111",
    "passwordpassword",
    "qwertyqwerty",
];

/// Rough entropy estimate — enough to steer the user, never a gate.
pub fn strength(pw: &str) -> Strength {
    if pw.len() < MIN_PASSWORD_LEN
        || COMMON.contains(&pw.to_ascii_lowercase().as_str())
        || trivial(pw)
    {
        return Strength::Weak;
    }
    let mut alphabet = 0u32;
    for (present, size) in [
        (pw.chars().any(|c| c.is_ascii_lowercase()), 26),
        (pw.chars().any(|c| c.is_ascii_uppercase()), 26),
        (pw.chars().any(|c| c.is_ascii_digit()), 10),
        (pw.chars().any(|c| !c.is_ascii_alphanumeric()), 33),
    ] {
        if present {
            alphabet += size;
        }
    }
    let bits = pw.len() as f32 * (alphabet.max(1) as f32).log2();
    match bits {
        b if b >= 80.0 => Strength::Strong,
        b if b >= 60.0 => Strength::Fair,
        _ => Strength::Weak,
    }
}

fn trivial(pw: &str) -> bool {
    let b = pw.as_bytes();
    b.len() > 1
        && (b.windows(2).all(|w| w[0] == w[1])
            || b.windows(2).all(|w| w[1] == w[0].wrapping_add(1))
            || b.windows(2).all(|w| w[0] == w[1].wrapping_add(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(password: &str, salt: &[u8]) -> Key {
        derive_key(&material(password, None), salt, &KdfParams::default()).unwrap()
    }

    #[test]
    fn roundtrip() {
        let salt = random_bytes::<SALT_LEN>();
        let k = key("correct horse battery staple", &salt);
        let (nonce, ct) = encrypt(b"attack at dawn", &k, b"header").unwrap();
        let pt = decrypt(&nonce, &ct, &k, b"header").unwrap();
        assert_eq!(pt, b"attack at dawn");
    }

    #[test]
    fn wrong_key_fails() {
        let salt = random_bytes::<SALT_LEN>();
        let k1 = key("aaa", &salt);
        let k2 = key("bbb", &salt);
        let (nonce, ct) = encrypt(b"secret", &k1, b"").unwrap();
        assert!(decrypt(&nonce, &ct, &k2, b"").is_err());
    }

    #[test]
    fn tamper_fails() {
        let salt = random_bytes::<SALT_LEN>();
        let k = key("pw", &salt);
        let (nonce, mut ct) = encrypt(b"secret", &k, b"").unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xFF;
        assert!(decrypt(&nonce, &ct, &k, b"").is_err());
    }

    #[test]
    fn salt_changes_key() {
        let s1 = random_bytes::<SALT_LEN>();
        let s2 = random_bytes::<SALT_LEN>();
        let k1 = key("pw", &s1);
        let k2 = key("pw", &s2);
        assert_ne!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn a_digest_names_without_saying() {
        let one = digest(b"/home/ana/Documents/divorce.pandora");
        assert_eq!(one, digest(b"/home/ana/Documents/divorce.pandora"));
        assert_eq!(one.len(), 32);
        assert!(one.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!one.contains("divorce") && !one.contains("home"));
        assert_ne!(one, digest(b"/home/ana/Documents/work.pandora"));
    }

    /// A file is named while it streams past, so the pieces must add up to the
    /// same digest as the whole.
    #[test]
    fn a_streamed_digest_matches_the_whole_one() {
        let whole = b"the first piece and the second piece";
        let mut hasher = Hasher::new();
        hasher.update(&whole[..18]);
        hasher.update(&whole[18..]);
        assert_eq!(hasher.finish(), digest(whole));
    }

    #[test]
    fn aad_mismatch_fails() {
        let salt = random_bytes::<SALT_LEN>();
        let k = key("pw", &salt);
        let (nonce, ct) = encrypt(b"secret", &k, b"header|argon2id").unwrap();
        assert!(decrypt(&nonce, &ct, &k, b"header|argon2i").is_err());
    }

    #[test]
    fn keyfile_changes_the_key() {
        let salt = random_bytes::<SALT_LEN>();
        let with = derive_key(
            &material("pw", Some(b"keyfile bytes")),
            &salt,
            &KdfParams::default(),
        )
        .unwrap();
        let without = key("pw", &salt);
        assert_ne!(with.as_ref(), without.as_ref());
        let again = derive_key(
            &material("pw", Some(b"keyfile bytes")),
            &salt,
            &KdfParams::default(),
        )
        .unwrap();
        assert_eq!(with.as_ref(), again.as_ref());
    }

    #[test]
    fn weak_passwords_are_called_weak() {
        for pw in [
            "",
            "short",
            "123456789012",
            "aaaaaaaaaaaa",
            "abcdefghijkl",
            "password1234",
            "qwertyuiop123",
            "correcthorsebatterystaple",
        ] {
            assert_eq!(strength(pw), Strength::Weak, "{pw}");
        }
    }

    #[test]
    fn length_and_variety_are_what_count() {
        assert_eq!(strength("correct horse battery staple"), Strength::Strong);
        assert_eq!(
            strength("Trebuchet!Mule7Vacuum"),
            Strength::Strong,
            "21 chars, four classes"
        );
        assert_eq!(
            strength("hunter2hunter2"),
            Strength::Fair,
            "14 chars, two classes"
        );
    }
}
