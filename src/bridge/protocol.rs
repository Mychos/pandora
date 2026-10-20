//! What the two peers say to each other: the browser's native-messaging framing —
//! a 4-byte length prefix in native byte order, then that many UTF-8 bytes of JSON,
//! as Chrome and Firefox document it — and the messages themselves.
//!
//! Both hops speak the same messages: the browser to `pandora-native-host`, and the
//! host to the app over a local socket. One set of types, written and read at both
//! ends, so the two cannot drift apart.
//!
//! One field rides only on the second hop: `caller`. The host reads it from its own
//! argv — the browser passes the caller's origin (Chromium) or the add-on id
//! (Firefox) — and adds it when it relays, because the app cannot see the browser's
//! process table. It is an envelope field, not part of a `Request`, so a request means
//! the same thing wherever it arrives.
//!
//! stdout carries the protocol and nothing else; logs go to stderr.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

/// A length prefix is an allocation size taken from a stream, so it is capped
/// rather than trusted — on either hop.
const MAX: usize = 1024 * 1024;

/// One message, or `None` at a clean end of input (the peer closed the port).
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

/// Write one answer, carrying back the pairing id the caller put on its request, so
/// both peers answer the same way and neither has to remember the convention.
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

/// Split the pairing id off a request, plus who the browser said was calling.
///
/// `id` rides back on the reply so the extension can pair it with its call: a
/// native-messaging port is one stream with no request/response framing of its own.
/// `caller` only appears on the hop the host makes to the app, which cannot see the
/// browser's process table; the host reads it from its own argv and adds it.
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
    /// The page the user is on. `url` is the whole address and wins when both
    /// arrive; `host` is what the first version of the extension sent.
    Match {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
    /// A login the user saved from the panel. Goes into the open box only.
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
    /// A password for a signup form. Never stored here — it rides back to the
    /// page the user asked from.
    Generate {
        #[serde(default)]
        length: Option<usize>,
    },
    Reveal {
        id: String,
    },
    /// The one-time seeds in the box, by name. No seed and no code: a listing never
    /// carries a secret.
    Codes,
    /// The code one seed is holding right now — derived, and about to expire. The
    /// seed itself never leaves the box.
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

/// An entry as the extension is allowed to see it: no password field exists here,
/// so none can leak through a listing or a match. `uri` is the stored URL that
/// claimed the page — the extension shows it so the user can see *why* an entry
/// offered itself.
#[derive(Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A one-time seed as the extension is allowed to see it: an id and a name. The
/// seed stays in the box; [`Request::Code`] derives the value for one id at a time,
/// after the user picks it — the same shape as `Candidate` and `Reveal`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CodeInfo {
    pub id: String,
    pub name: String,
}

/// Both peers send this one, so it is written and read on either hop.
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
        /// Whether this peer could open a box if asked — the host says yes only when
        /// the user has allowed unlocking from the browser, which is off by default.
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
