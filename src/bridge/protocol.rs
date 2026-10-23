//! The browser's framing — a 4-byte native-order length prefix, then UTF-8 JSON, as
//! Chrome and Firefox document it — and the messages both hops share.
//! `caller` rides only on the host-to-app hop, read from argv.
//! stdout carries the protocol and nothing else; logs go to stderr.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

/// An allocation size read from a stream, so capped rather than trusted, on either hop.
const MAX: usize = 1024 * 1024;

/// One message, or `None` at a clean end of input.
pub fn read<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_ne_bytes(len) as usize;
    if n > MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message larger than 1 MiB",
        ));
    }
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub fn write<W: Write>(writer: &mut W, msg: &[u8]) -> io::Result<()> {
    let n = u32::try_from(msg.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message larger than 4 GiB"))?;
    writer.write_all(&n.to_ne_bytes())?;
    writer.write_all(msg)?;
    writer.flush()
}

/// Write one answer, carrying the request's pairing id back, so both peers agree.
pub fn reply<W: Write>(writer: &mut W, id: Option<u64>, response: &Response) -> io::Result<()> {
    let mut body = serde_json::to_value(response)
        .unwrap_or_else(|_| serde_json::json!({ "type": "error", "message": "serialize" }));
    if let (Some(object), Some(id)) = (body.as_object_mut(), id) {
        object.insert("_id".to_owned(), Value::from(id));
    }
    let json = serde_json::to_vec(&body)
        .unwrap_or_else(|_| br#"{"type":"error","message":"serialize"}"#.to_vec());
    write(writer, &json)
}

/// Split the pairing id and the caller off a request: a native-messaging port has no
/// framing of its own, and `caller` only exists on the host-to-app hop.
#[derive(Debug)]
pub struct Envelope {
    pub id: Option<u64>,
    pub caller: Option<String>,
    pub request: Result<Request, String>,
}

pub fn parse(bytes: &[u8]) -> Envelope {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(e) => {
            return Envelope {
                id: None,
                caller: None,
                request: Err(format!("bad request: {e}")),
            }
        }
    };
    Envelope {
        id: value.get("_id").and_then(Value::as_u64),
        caller: value
            .get("caller")
            .and_then(Value::as_str)
            .map(str::to_owned),
        request: serde_json::from_value(value).map_err(|e| format!("bad request: {e}")),
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ListBoxes,
    Unlock {
        path: String,
        password: String,
    },
    Lock,
    Status,
    Entries,
    /// `url` wins when both arrive; `host` is what the first extension version sent.
    Match {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
    /// A login saved from the panel; goes into the open box only.
    Create {
        name: String,
        username: String,
        password: String,
        url: String,
    },
    /// What the panel changed; a field left out keeps what the entry had.
    Update {
        id: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    /// A password for a signup form: never stored, it rides back to the page.
    Generate {
        #[serde(default)]
        length: Option<usize>,
    },
    Reveal {
        id: String,
    },
    /// The passkeys a site may use; a listing never carries a key.
    PasskeyList {
        rp_id: String,
    },
    /// A new credential for one site, minted by the host and stored in the open box.
    PasskeyCreate {
        rp_id: String,
        /// The account name the page showed; it names the record too.
        #[serde(default)]
        user_name: String,
    },
    /// One assertion for one credential, and only after the user picked it.
    PasskeyGet {
        rp_id: String,
        /// The credential the user picked, base64 as the page sent it.
        credential_id: String,
        /// The challenge the page asked about, base64url: checked against the client data.
        challenge: String,
        /// The `clientDataJSON` the page will post back, base64.
        client_data: String,
    },
    /// The seeds in the box, by name: a listing never carries a secret.
    Codes,
    /// The code one seed holds now, derived and about to expire; the seed stays put.
    Code {
        id: String,
    },
    Refresh,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BoxInfo {
    pub path: String,
    pub name: String,
}

/// An entry as the extension may see it: no password field exists here, and `uri` is
/// the stored URL that claimed the page, so the user sees why it was offered.
#[derive(Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A passkey as the extension may see it: no key and no seed, only what a page needs
/// to offer the credential and the handle it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyInfo {
    pub id: String,
    pub name: String,
    pub rp_id: String,
    /// COSE algorithm: -7 ES256, -8 EdDSA.
    pub alg: i32,
    /// Base64 of the credential id, which the site already knows: it is what
    /// `allowCredentials` names, not a secret.
    pub credential_id: String,
    /// Base64 of the account handle the site gave at creation.
    pub user_handle: String,
}

/// A seed as the extension may see it: id and name. [`Request::Code`] derives the
/// value for one id after the user picks it.
#[derive(Debug, Serialize, Deserialize)]
pub struct CodeInfo {
    pub id: String,
    pub name: String,
}

/// Both peers send this one, on either hop.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Boxes {
        boxes: Vec<BoxInfo>,
    },
    Unlocked {
        name: String,
        count: usize,
    },
    Ok,
    Status {
        unlocked: bool,
        /// The host says yes only when the user allowed browser unlocks (off by default).
        unlockable: bool,
        name: Option<String>,
        count: usize,
    },
    Entries {
        entries: Vec<Candidate>,
    },
    /// The entry the panel just wrote, so it can show it without a second call.
    Saved {
        entry: Candidate,
    },
    Passkeys {
        passkeys: Vec<PasskeyInfo>,
    },
    /// A credential just minted: the attestation object and what it holds.
    PasskeyCreated {
        entry: PasskeyInfo,
        /// Base64 of the raw credential id the page must keep.
        credential_id: String,
        /// Base64 of the `fmt: "none"` attestation object, CBOR.
        attestation: String,
    },
    /// One assertion, over a client data the page already has.
    PasskeyAssertion {
        /// Base64 of the credential that signed.
        credential_id: String,
        /// Base64 of `authenticatorData`.
        auth_data: String,
        /// Base64 of the signature (DER for ES256, raw for EdDSA).
        signature: String,
        /// Base64 of the handle, so the page can say who signed in.
        user_handle: String,
        counter: u32,
    },
    Generated {
        password: String,
    },
    Secret {
        username: String,
        password: String,
    },
    Codes {
        codes: Vec<CodeInfo>,
    },
    Code {
        code: String,
        /// Seconds until this code is replaced by the next one.
        remaining: u64,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_survives_the_wire() {
        let mut wire = Vec::new();
        write(&mut wire, br#"{"type":"status"}"#).unwrap();
        write(&mut wire, b"second").unwrap();

        let mut reader: &[u8] = &wire;
        assert_eq!(read(&mut reader).unwrap().unwrap(), br#"{"type":"status"}"#);
        assert_eq!(read(&mut reader).unwrap().unwrap(), b"second");
        assert!(read(&mut reader).unwrap().is_none(), "clean end of input");
    }

    #[test]
    fn a_length_prefix_past_the_cap_is_refused_before_allocating() {
        let mut wire = ((MAX + 1) as u32).to_ne_bytes().to_vec();
        wire.extend_from_slice(b"x");
        let mut reader: &[u8] = &wire;
        assert!(read(&mut reader).is_err());
    }

    #[test]
    fn the_pairing_id_is_split_off_the_request() {
        let envelope = parse(br#"{"_id":7,"type":"status"}"#);
        assert_eq!(envelope.id, Some(7));
        assert!(matches!(envelope.request, Ok(Request::Status)));

        let envelope = parse(br#"{"type":"nonsense"}"#);
        assert!(
            envelope.request.is_err(),
            "an unknown request is refused, not guessed"
        );
    }

    #[test]
    fn the_caller_rides_alongside_the_request() {
        let envelope = parse(br#"{"_id":1,"caller":"chrome-extension://abc/","type":"status"}"#);
        assert_eq!(envelope.caller.as_deref(), Some("chrome-extension://abc/"));
        assert!(
            !serde_json::to_string(&Response::Ok)
                .unwrap()
                .contains("caller"),
            "the caller is not part of an answer"
        );
    }
}
