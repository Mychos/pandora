//! WebAuthn, host side. The box holds a raw private key and the host does the rest:
//! the browser never runs a signature, never sees a key and never sees the CBOR.
//!
//! Only the two algorithms a platform authenticator is expected to offer are here —
//! ES256 (the default) and EdDSA — and the attestation is `none`, exactly as a
//! passkey stored on one device is attested. A relying party that demands a
//! hardware attestation is not served by this, and is not meant to be.

use crate::bridge::protocol::{PasskeyInfo, Response};
use crate::vault::codec;
use crate::vault::crypto::random_bytes;
use crate::vault::model::{now_secs, Passkey, Vault};
use ciborium::value::Value;
use ed25519_dalek::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// COSE algorithm identifiers.
pub const ES256: i32 = -7;
pub const EDDSA: i32 = -8;

/// `authenticatorData` flags: user present, user verified, attested data included.
const UP: u8 = 0x01;
const UV: u8 = 0x04;
const AT: u8 = 0x40;

/// A 16-byte all-zero AAGUID: no hardware, no attestation, which is what `none` says.
const AAGUID: [u8; 16] = [0; 16];

/// The passkeys a site may use: the id and the handle a page needs, never a key.
pub fn list(vault: &Vault, rp_id: &str) -> Vec<PasskeyInfo> {
    vault
        .passkeys
        .iter()
        .filter(|it| it.rp_id == rp_id)
        .map(info)
        .collect()
}

fn info(it: &Passkey) -> PasskeyInfo {
    PasskeyInfo {
        id: it.id.clone(),
        name: it.name.clone(),
        rp_id: it.rp_id.clone(),
        alg: it.alg,
        credential_id: codec::display(&it.credential_id).to_string(),
        user_handle: codec::display(&it.user_handle).to_string(),
    }
}

/// Mint a credential for the site and keep it in the box. The private key never leaves
/// this call: only the attestation goes back.
pub fn create_in(vault: &mut Vault, rp_id: &str, user_name: &str) -> Response {
    let name = if user_name.trim().is_empty() {
        rp_id.to_owned()
    } else {
        user_name.trim().to_owned()
    };
    let minted = match create(rp_id, None) {
        Ok(minted) => minted,
        Err(message) => return Response::Error { message },
    };
    let mut it = Passkey::new(now_secs(), rp_id);
    it.name = name;
    it.alg = minted.alg;
    it.credential_id = minted.credential_id.clone();
    it.user_handle = minted.user_handle.clone();
    it.private_key = minted.private.clone();
    let entry = info(&it);
    vault.passkeys.push(it);
    let credential_id = codec::display(&minted.credential_id).to_string();
    let attestation = codec::display(&minted.attestation).to_string();
    Response::PasskeyCreated {
        entry,
        credential_id,
        attestation,
    }
}

/// One assertion for one credential of one site. The counter moves, so a relying party
/// that watches it sees each assertion as new.
pub fn assert_in(
    vault: &mut Vault,
    rp_id: &str,
    credential_id: &[u8],
    client_data: &[u8],
) -> Response {
    let Some(it) = vault
        .passkeys
        .iter_mut()
        .find(|it| it.rp_id == rp_id && it.credential_id.as_slice() == credential_id)
    else {
        return Response::Error {
            message: "no such passkey".to_owned(),
        };
    };
    let counter = it.counter.saturating_add(1);
    let assertion = match assert(it, client_data, true, counter) {
        Ok(assertion) => assertion,
        Err(message) => return Response::Error { message },
    };
    it.counter = counter;
    it.touched(now_secs());
    let credential_id = codec::display(&it.credential_id).to_string();
    let auth_data = codec::display(&assertion.auth_data).to_string();
    let signature = codec::display(&assertion.signature).to_string();
    let user_handle = codec::display(&it.user_handle).to_string();
    Response::PasskeyAssertion {
        credential_id,
        auth_data,
        signature,
        user_handle,
        counter,
    }
}

/// A credential the host just minted, ready to be written into the box.
pub struct Minted {
    pub alg: i32,
    pub credential_id: Zeroizing<Vec<u8>>,
    pub user_handle: Zeroizing<Vec<u8>>,
    /// The raw key the box keeps: a P-256 scalar or an Ed25519 seed.
    pub private: Zeroizing<Vec<u8>>,
    /// `authenticatorData` with the attested credential data inside it.
    pub auth_data: Vec<u8>,
    /// The `fmt: "none"` attestation object, CBOR.
    pub attestation: Vec<u8>,
}

/// What an assertion hands back for a page to post.
pub struct Assertion {
    pub auth_data: Vec<u8>,
    pub signature: Vec<u8>,
}

/// A passkey with a fresh ES256 credential; the shape this app writes.
pub fn create(rp_id: &str, user_handle: Option<&[u8]>) -> Result<Minted, String> {
    create_with(rp_id, user_handle, ES256)
}

/// The same, for the algorithm a relying party asked for.
pub fn create_with(rp_id: &str, user_handle: Option<&[u8]>, alg: i32) -> Result<Minted, String> {
    if rp_id.trim().is_empty() {
        return Err("a passkey needs a site".to_owned());
    }
    let user_handle = user_handle
        .map(|handle| Zeroizing::new(handle.to_vec()))
        .unwrap_or_else(|| Zeroizing::new(random_bytes::<32>().to_vec()));
    let credential_id = Zeroizing::new(random_bytes::<32>().to_vec());

    let (private, cose) = match alg {
        ES256 => {
            let private = Zeroizing::new(random_bytes::<32>().to_vec());
            let signing = p256_signing(&private)?;
            let point = signing.verifying_key().to_sec1_point(false);
            let key = cose_key(
                alg,
                point.x().ok_or("the point has no x")?.as_slice(),
                Some(point.y().ok_or("the point has no y")?.as_slice()),
            );
            (private, key)
        }
        EDDSA => {
            let private = Zeroizing::new(random_bytes::<32>().to_vec());
            let signing = ed25519_signing(&private)?;
            let public = signing.verifying_key().to_bytes();
            (private, cose_key(alg, &public, None))
        }
        _ => return Err(format!("algorithm {alg} is not one this host signs with")),
    };

    let mut auth_data = authenticator(rp_id, UP | UV | AT, 0);
    auth_data.extend_from_slice(&AAGUID);
    auth_data.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
    auth_data.extend_from_slice(credential_id.as_slice());
    auth_data.extend_from_slice(&cose);

    Ok(Minted {
        alg,
        credential_id,
        user_handle,
        private,
        attestation: attestation(&auth_data),
        auth_data,
    })
}

/// `authenticatorData` for one assertion: the site's hash, the flags and the counter.
pub fn authenticator(rp_id: &str, flags: u8, counter: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(37);
    out.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    out.push(flags);
    out.extend_from_slice(&counter.to_be_bytes());
    out
}

/// Sign `SHA-256(authenticatorData || SHA-256(clientDataJSON))` with the key the box
/// holds. `uv` is the unlock the app has: it is the user verification this host can
/// honestly claim.
pub fn assert(
    passkey: &Passkey,
    client_data: &[u8],
    uv: bool,
    counter: u32,
) -> Result<Assertion, String> {
    if passkey.rp_id.trim().is_empty() {
        return Err("the passkey names no site".to_owned());
    }
    let flags = UP | if uv { UV } else { 0 };
    let auth_data = authenticator(&passkey.rp_id, flags, counter);
    let client_data_hash = Sha256::digest(client_data);
    let mut signed = auth_data.clone();
    signed.extend_from_slice(&client_data_hash);
    let signature = sign(passkey, &signed)?;
    Ok(Assertion {
        auth_data,
        signature,
    })
}

/// The COSE public key of the private key a passkey holds; the box never needs it, but
/// a test does, and so does a page that asks what the credential looks like.
pub fn public_key(passkey: &Passkey) -> Result<Vec<u8>, String> {
    match passkey.alg {
        ES256 => {
            let signing = p256_signing(&passkey.private_key)?;
            let point = signing.verifying_key().to_sec1_point(false);
            Ok(cose_key(
                passkey.alg,
                point.x().ok_or("the point has no x")?.as_slice(),
                Some(point.y().ok_or("the point has no y")?.as_slice()),
            ))
        }
        EDDSA => {
            let signing = ed25519_signing(&passkey.private_key)?;
            Ok(cose_key(
                passkey.alg,
                &signing.verifying_key().to_bytes(),
                None,
            ))
        }
        alg => Err(format!("algorithm {alg} is not one this host signs with")),
    }
}

fn sign(passkey: &Passkey, data: &[u8]) -> Result<Vec<u8>, String> {
    match passkey.alg {
        ES256 => {
            let signing = p256_signing(&passkey.private_key)?;
            let signature: Signature = signing.sign(data);
            Ok(signature.to_der().as_bytes().to_vec())
        }
        EDDSA => {
            let signing = ed25519_signing(&passkey.private_key)?;
            Ok(signing.sign(data).to_bytes().to_vec())
        }
        alg => Err(format!("algorithm {alg} is not one this host signs with")),
    }
}

/// Whether a signature is the passkey's, over exactly these bytes. The host does not
/// need it to answer a page; it is how the box proves itself in a test.
pub fn verify(passkey: &Passkey, data: &[u8], signature: &[u8]) -> bool {
    match passkey.alg {
        ES256 => {
            let Ok(signing) = p256_signing(&passkey.private_key) else {
                return false;
            };
            let Ok(signature) = Signature::from_der(signature) else {
                return false;
            };
            p256::ecdsa::signature::Verifier::verify(signing.verifying_key(), data, &signature)
                .is_ok()
        }
        EDDSA => {
            let Ok(signing) = ed25519_signing(&passkey.private_key) else {
                return false;
            };
            let Ok(signature) = ed25519_dalek::Signature::from_slice(signature) else {
                return false;
            };
            ed25519_dalek::Verifier::verify(&signing.verifying_key(), data, &signature).is_ok()
        }
        _ => false,
    }
}

fn p256_signing(private: &[u8]) -> Result<SigningKey, String> {
    SigningKey::from_slice(private).map_err(|_| "this passkey's key does not read".to_owned())
}

fn ed25519_signing(private: &[u8]) -> Result<ed25519_dalek::SigningKey, String> {
    let seed: [u8; 32] = private
        .try_into()
        .map_err(|_| "this passkey's key does not read".to_owned())?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

/// A COSE_Key, as WebAuthn puts it in the attested credential data: an EC2 point for
/// ES256, an OKP point for EdDSA.
fn cose_key(alg: i32, x: &[u8], y: Option<&[u8]>) -> Vec<u8> {
    let mut map: Vec<(Value, Value)> = Vec::with_capacity(5);
    let kty = if alg == EDDSA { 1 } else { 2 };
    let crv = if alg == EDDSA { 6 } else { 1 };
    map.push((Value::Integer(1.into()), Value::Integer(kty.into())));
    map.push((Value::Integer(3.into()), Value::Integer(alg.into())));
    map.push((Value::Integer((-1).into()), Value::Integer(crv.into())));
    map.push((Value::Integer((-2).into()), Value::Bytes(x.to_vec())));
    if let Some(y) = y {
        map.push((Value::Integer((-3).into()), Value::Bytes(y.to_vec())));
    }
    let mut out = Vec::new();
    ciborium::into_writer(&Value::Map(map), &mut out).expect("CBOR is written into memory");
    out
}

/// The attestation object a `fmt: "none"` credential carries.
fn attestation(auth_data: &[u8]) -> Vec<u8> {
    let map = Value::Map(vec![
        (
            Value::Text("fmt".to_owned()),
            Value::Text("none".to_owned()),
        ),
        (Value::Text("attStmt".to_owned()), Value::Map(Vec::new())),
        (
            Value::Text("authData".to_owned()),
            Value::Bytes(auth_data.to_vec()),
        ),
    ]);
    let mut out = Vec::new();
    ciborium::into_writer(&map, &mut out).expect("CBOR is written into memory");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::now_secs;
    use ciborium::value::Value as Cbor;

    fn stored(minted: &Minted, rp_id: &str) -> Passkey {
        let mut it = Passkey::new(now_secs(), rp_id);
        it.alg = minted.alg;
        it.credential_id = minted.credential_id.clone();
        it.user_handle = minted.user_handle.clone();
        it.private_key = minted.private.clone();
        it
    }

    #[test]
    fn a_new_credential_attests_nothing_and_names_its_site() {
        let minted = create("example.com", None).unwrap();
        assert_eq!(minted.alg, ES256);
        assert_eq!(minted.credential_id.len(), 32);
        assert_eq!(
            minted.user_handle.len(),
            32,
            "a handle is minted when none is given"
        );

        let object: Cbor = ciborium::from_reader(minted.attestation.as_slice()).unwrap();
        let Cbor::Map(fields) = object else {
            panic!("an attestation is a map");
        };
        let get = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| matches!(key, Cbor::Text(text) if text == name))
                .map(|(_, value)| value.clone())
        };
        assert_eq!(get("fmt"), Some(Cbor::Text("none".to_owned())));
        assert_eq!(get("attStmt"), Some(Cbor::Map(Vec::new())));
        let Some(Cbor::Bytes(auth)) = get("authData") else {
            panic!("the attestation carries the authenticator data");
        };
        assert_eq!(auth.len(), minted.auth_data.len());
        assert_eq!(&auth[..32], Sha256::digest(b"example.com").as_slice());
        assert_eq!(
            auth[32] & (UP | UV | AT),
            UP | UV | AT,
            "present, verified, attested"
        );
        assert_eq!(&auth[37..53], &AAGUID);
        let id_len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
        assert_eq!(id_len, 32);
        assert_eq!(&auth[55..55 + id_len], minted.credential_id.as_slice());

        let cose: Cbor = ciborium::from_reader(&auth[55 + id_len..]).unwrap();
        let Cbor::Map(key) = cose else {
            panic!("a COSE key is a map");
        };
        assert!(key.contains(&(Cbor::Integer(1.into()), Cbor::Integer(2.into()))));
        assert!(key.contains(&(Cbor::Integer(3.into()), Cbor::Integer((-7).into()))));
        assert!(key.contains(&(Cbor::Integer((-1).into()), Cbor::Integer(1.into()))));
        assert_eq!(key.len(), 5, "an EC2 key is kty, alg, crv, x and y");
    }

    #[test]
    fn an_assertion_verifies_over_the_authenticator_data_and_the_client_data() {
        let minted = create("example.com", Some(b"handle")).unwrap();
        let passkey = stored(&minted, "example.com");
        let client_data = br#"{"type":"webauthn.get","challenge":"AAAA"}"#;

        let assertion = assert(&passkey, client_data, true, 1).unwrap();
        assert_eq!(assertion.auth_data[32], UP | UV);
        assert_eq!(&assertion.auth_data[33..37], &1u32.to_be_bytes());

        let mut signed = assertion.auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(client_data));
        assert!(
            verify(&passkey, &signed, &assertion.signature),
            "the signature is over both halves"
        );
        assert!(
            !verify(&passkey, &assertion.auth_data, &assertion.signature),
            "the client data is part of what was signed"
        );
    }

    /// The site is bound into the first thirty-two bytes: an assertion made for one
    /// origin cannot be replayed at another.
    #[test]
    fn an_assertion_names_only_its_own_site() {
        let minted = create("example.com", None).unwrap();
        let passkey = stored(&minted, "example.com");
        let client_data = b"{}";
        let assertion = assert(&passkey, client_data, true, 0).unwrap();
        let mut signed = assertion.auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(client_data));
        assert!(verify(&passkey, &signed, &assertion.signature));

        let mut other = assertion.auth_data.clone();
        other[..32].copy_from_slice(&Sha256::digest(b"evil.example"));
        let mut moved = other;
        moved.extend_from_slice(&Sha256::digest(client_data));
        assert!(!verify(&passkey, &moved, &assertion.signature));
    }

    #[test]
    fn eddsa_is_the_other_algorithm_this_host_signs_with() {
        let minted = create_with("example.com", None, EDDSA).unwrap();
        assert_eq!(minted.alg, EDDSA);
        let passkey = stored(&minted, "example.com");
        let assertion = assert(&passkey, b"{}", false, 3).unwrap();
        assert_eq!(assertion.signature.len(), 64, "Ed25519 signs raw");
        let mut signed = assertion.auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(b"{}"));
        assert!(verify(&passkey, &signed, &assertion.signature));

        let cose: Cbor = ciborium::from_reader(public_key(&passkey).unwrap().as_slice()).unwrap();
        let Cbor::Map(key) = cose else {
            panic!("a COSE key is a map");
        };
        assert!(
            key.contains(&(Cbor::Integer(1.into()), Cbor::Integer(1.into()))),
            "OKP"
        );
        assert!(
            key.contains(&(Cbor::Integer((-1).into()), Cbor::Integer(6.into()))),
            "Ed25519"
        );
        let Some((_, Cbor::Bytes(x))) = key
            .iter()
            .find(|(k, _)| *k == Cbor::Integer((-2).into()))
            .cloned()
        else {
            panic!("the point is x");
        };
        assert_eq!(x.len(), 32);
    }

    /// The two calls a browser makes, over a real box: the credential lands in it, the
    /// listing says so without a key, and an assertion moves the counter.
    #[test]
    fn a_credential_is_stored_and_answers_only_its_own_site() {
        let mut vault = Vault::new("Box");
        let Response::PasskeyCreated {
            entry,
            credential_id,
            attestation,
        } = create_in(&mut vault, "example.com", "ana@example.com")
        else {
            panic!("a credential comes back");
        };
        assert_eq!(vault.passkeys.len(), 1);
        assert_eq!(vault.passkeys[0].name, "ana@example.com");
        assert_eq!(entry.rp_id, "example.com");
        assert_eq!(vault.passkeys[0].counter, 0);
        assert!(!attestation.is_empty());

        let listed = list(&vault, "example.com");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "ana@example.com");
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains(&*codec::display(&vault.passkeys[0].private_key).to_string()));

        let id = codec::decode(&credential_id).unwrap();
        let answer = assert_in(&mut vault, "example.com", &id, br#"{"challenge":"AAAA"}"#);
        let Response::PasskeyAssertion {
            auth_data,
            signature,
            counter,
            ..
        } = answer
        else {
            panic!("an assertion comes back: {answer:?}");
        };
        assert_eq!(counter, 1);
        let auth = codec::decode(&auth_data).unwrap();
        assert_eq!(&auth[..32], Sha256::digest(b"example.com").as_slice());
        let mut signed = codec::decode(&auth_data).unwrap();
        signed.extend_from_slice(&Sha256::digest(br#"{"challenge":"AAAA"}"#));
        assert!(verify(
            &vault.passkeys[0],
            &signed,
            &codec::decode(&signature).unwrap()
        ));

        // Another site is another credential: nothing answers for it.
        let answer = assert_in(&mut vault, "evil.example", &id, b"{}");
        assert!(matches!(answer, Response::Error { .. }), "{answer:?}");
        assert!(matches!(
            assert_in(&mut vault, "example.com", b"not the id", b"{}"),
            Response::Error { .. }
        ));
    }

    #[test]
    fn a_key_that_does_not_read_or_an_algorithm_we_do_not_sign_is_refused() {
        let mut passkey = Passkey::new(1, "example.com");
        passkey.private_key = Zeroizing::new(vec![0; 3]);
        assert!(assert(&passkey, b"{}", true, 0).is_err());
        passkey.alg = -257;
        assert!(assert(&passkey, b"{}", true, 0).is_err());
    }
}
