//! Opening a box with the device's own reader — a fingerprint, a face — instead
//! of typing the master password.
//!
//! A finger cannot produce key material: no platform hands out entropy read from
//! one. So what the finger unlocks is the platform's keystore, and what the
//! keystore holds is a box's *derived key*. That key came out of the KDF the
//! first time the password was typed, so an unlock here runs no Argon2 at all:
//! the file's own header plus that key is everything [`store::open_keyed`] needs.
//!
//! The key is never written down in the clear. The platform encrypts it under a
//! keystore key that only a live finger can use — Android demands user
//! authentication for every decryption, and throws the key away when a new
//! fingerprint is enrolled, which is why a re-enrolled finger falls back to the
//! password once. What lands in the app's data dir is only the ciphertext, one
//! entry per box.
//!
//! A seal is a convenience, never a second door into the vault: it is exactly as
//! strong as the device's keystore, and a box whose password was rotated (a new
//! salt, so a new key) no longer opens with the sealed one — the AEAD tag refuses
//! it and the password is asked for again.

use crate::features::settings::model::box_key;
use crate::vault::crypto::Key;
use crate::vault::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod other;

#[cfg(target_os = "android")]
use android as platform;
#[cfg(not(target_os = "android"))]
use other as platform;

/// What a platform says when the user backs out of the prompt: nothing failed,
/// nothing changed, and the app says nothing about it.
pub const CANCELED: &str = "fingerprint not confirmed";

/// Is there a reader on this device at all? False on a desktop, and false on a
/// phone with no lock screen, which is where the whole feature starts.
pub fn available() -> bool {
    platform::available()
}

/// Is this box set up to open with a finger?
pub fn sealed(path: &Path) -> bool {
    read(&dir()).boxes.contains_key(&key_of(path))
}

/// Hand the derived key to the platform's keystore and keep only the ciphertext.
/// This is what a successful password unlock does when the user asked for it.
pub fn seal(path: &Path, key: &Key) -> Result<(), String> {
    let blob = platform::seal(key)?;
    let mut all = read(&dir());
    all.boxes.insert(key_of(path), blob);
    write(&dir(), &all);
    Ok(())
}

/// The key a live finger just released. A box whose password changed no longer
/// decrypts with it — the caller falls back to asking for the password.
pub fn unseal(path: &Path) -> Result<Key, String> {
    let blob = read(&dir()).boxes.get(&key_of(path)).cloned();
    let Some(blob) = blob else {
        return Err("this box has no fingerprint set up".to_owned());
    };
    platform::unseal(&blob)
}

/// Drop the seal: the user turned the feature off, or the box is gone.
pub fn forget(path: &Path) {
    let mut all = read(&dir());
    if all.boxes.remove(&key_of(path)).is_some() {
        write(&dir(), &all);
    }
}

/// One entry per box path, keyed the same way the failed-unlock counters are, so
/// two spellings of one file share one seal.
fn key_of(path: &Path) -> String {
    box_key(&path.display().to_string())
}

/// What the app keeps: ciphertext only, and the platform's own key to open it.
///
/// `pub(crate)` with [`write`] so the contract test can put a seal beside the
/// other two side files and check what none of them may say.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Sealed {
    #[serde(default)]
    pub(crate) boxes: BTreeMap<String, String>,
}

fn dir() -> PathBuf {
    store::data_dir()
}

fn file(dir: &Path) -> PathBuf {
    dir.join("biometric.json")
}

fn read(dir: &Path) -> Sealed {
    let Ok(bytes) = fs::read(file(dir)) else {
        return Sealed::default();
    };
    let mut sealed: Sealed = serde_json::from_slice(&bytes).unwrap_or_default();
    // A seal written by a build that keyed it by the box's path is re-keyed to the
    // digest — it is the same box, and the file must stop naming it — and written
    // back so nothing in it still spells a path.
    let mut moved = false;
    sealed.boxes = sealed
        .boxes
        .into_iter()
        .map(|(k, v)| {
            let key = crate::features::settings::model::adopt_key(&k);
            moved |= key != k;
            (key, v)
        })
        .collect();
    if moved {
        write(dir, &sealed);
    }
    sealed
}

/// Best effort: losing the file only costs the convenience, never a box — and a
/// failure here must never take an unlock with it.
pub(crate) fn write(dir: &Path, sealed: &Sealed) {
    let Ok(bytes) = serde_json::to_vec(sealed) else {
        return;
    };
    if fs::create_dir_all(dir).is_ok() {
        let _ = store::write_private(&file(dir), &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pandora-biometric-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_box_is_sealed_by_its_path_and_forgotten_by_it() {
        let dir = tmp("sealed");
        let box_a = dir.join("a.pandora");
        let box_b = dir.join("b.pandora");

        let mut all = read(&dir);
        all.boxes.insert(key_of(&box_a), "ciphertext".to_owned());
        write(&dir, &all);

        let sealed = read(&dir);
        assert!(sealed.boxes.contains_key(&key_of(&box_a)));
        assert!(!sealed.boxes.contains_key(&key_of(&box_b)));

        let mut all = read(&dir);
        all.boxes.remove(&key_of(&box_a));
        write(&dir, &all);
        assert!(
            read(&dir).boxes.is_empty(),
            "the entry is gone, not left behind"
        );
    }

    /// The file is only ever ciphertext: a device that cannot open it is a
    /// keystore question, never a vault one.
    #[test]
    fn an_unreadable_file_costs_the_seal_and_nothing_else() {
        let dir = tmp("broken");
        fs::write(file(&dir), b"{ not json").unwrap();
        assert!(read(&dir).boxes.is_empty());
    }

    /// A seal written by a build that keyed it by the box's path is re-keyed to
    /// the digest on the way in, and the file stops spelling the path.
    #[test]
    fn a_path_keyed_seal_is_re_keyed_and_rewritten() {
        let dir = tmp("legacy-seal");
        let box_path = dir.join("legacy.pandora");
        let path = box_path.display().to_string();
        fs::write(
            file(&dir),
            format!(r#"{{"boxes":{{"{path}":"ciphertext"}}}}"#),
        )
        .unwrap();

        let sealed = read(&dir);
        assert_eq!(sealed.boxes[&key_of(&box_path)], "ciphertext");
        let text = fs::read_to_string(file(&dir)).unwrap();
        assert!(
            !text.contains("legacy.pandora"),
            "the path left the seal file: {text}"
        );
    }
}
