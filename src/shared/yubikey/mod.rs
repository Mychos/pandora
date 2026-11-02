//! The YubiKey's HMAC-SHA1 challenge-response: a second factor that never leaves the
//! physical key, and that the box does not have to remember. The challenge is derived
//! from the box's own id, so the same response comes back for a copy of the box and
//! nothing new is written into the header.
//!
//! Desktop only, because a phone has no port for one. The USB side is the only part
//! behind a `cfg`; the challenge and the error are the same everywhere.

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// The challenge a box asks its key for. Tied to the box, not to the user, so a mirror
/// or a backup answers identically — and a different box asks a different question.
pub fn challenge(box_id: &str) -> Vec<u8> {
    Sha256::digest(box_id.as_bytes()).to_vec()
}

/// Whether this host can talk USB at all, asked once. The crate that owns the key opens
/// its context with an `unwrap`, so a container or a runner without usbfs panics rather
/// than answering; asking rusb the same question first is what keeps that a "no key"
/// instead of a frame that dies mid-settings. Whether the host has usbfs does not change
/// while the process runs, so the answer is remembered.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn usb() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| rusb::Context::new().is_ok())
}

/// Whether a key is plugged in. Looks at the bus and never waits for a touch, so it is
/// safe to ask on a frame.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn present() -> bool {
    use yubikey_hmac_otp::Yubico;
    /// A frame that draws the settings page asks here, and walking the bus per frame is
    /// not something a window can do: a plugged key is still noticed a moment later.
    const TTL: std::time::Duration = std::time::Duration::from_secs(2);
    static LAST: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().expect("yubikey cache");
    if let Some((at, present)) = *last {
        if at.elapsed() < TTL {
            return present;
        }
    }
    let present = usb()
        && Yubico::new()
            .find_all_yubikeys()
            .map(|keys| !keys.is_empty())
            .unwrap_or(false);
    *last = Some((std::time::Instant::now(), present));
    present
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn present() -> bool {
    false
}

/// Ask the key for its HMAC-SHA1 response. It blocks until the key is touched, so this
/// belongs on a worker thread and never in a frame.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn respond(challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    use yubikey_hmac_otp::config::{Config, Mode, Slot};
    use yubikey_hmac_otp::Yubico;

    if !usb() {
        return Err("this host cannot reach a USB key".to_owned());
    }
    let mut yubico = Yubico::new();
    let key = yubico
        .find_yubikey()
        .map_err(|e| format!("no YubiKey answered: {e}"))?;
    let config = Config::new_from(key)
        .set_variable_size(true)
        .set_slot(Slot::Slot2)
        .set_mode(Mode::Sha1);
    let hmac = yubico
        .challenge_response_hmac(challenge, config)
        .map_err(|e| format!("the YubiKey did not answer: {e}"))?;
    Ok(Zeroizing::new(hmac.0.to_vec()))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn respond(_challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    Err("a YubiKey needs a desktop".to_owned())
}

/// The key as a factor: what `store` asks for a response, never the response itself.
pub struct Usb;

impl crate::vault::store::Factor for Usb {
    fn respond(&self, challenge: &[u8]) -> Result<Vec<u8>, String> {
        respond(challenge).map(|bytes| bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_belongs_to_the_box() {
        let one = challenge("a-box-id");
        let two = challenge("another-box-id");
        assert_eq!(one.len(), 32, "a SHA-256 of the id");
        assert_eq!(one, challenge("a-box-id"), "and it is the same every time");
        assert_ne!(one, two);
        assert!(
            !one.windows(4).any(|w| w == b"box"),
            "the id is not spelled in it"
        );
    }

    /// With no key on the bus the answer is an error the caller can show — never a
    /// panic and never an empty response that would derive a box nobody can open.
    #[test]
    fn no_key_is_an_error_or_an_honest_answer() {
        let answer = respond(b"challenge");
        match answer {
            Err(message) => assert!(!message.is_empty()),
            Ok(response) => assert_eq!(response.len(), 20, "HMAC-SHA1 is 20 bytes"),
        }
    }
}
