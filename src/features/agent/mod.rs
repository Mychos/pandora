//! The SSH agent: the box's Ed25519 keys, offered on a Unix socket so `git push` and
//! `ssh` can use them without the key ever existing as a file.
//!
//! Only the keys the user marked `allowed` are listed and only they sign. Nothing is
//! served while the box is locked: the snapshot the listener reads is emptied when the
//! session goes, so a request that arrives after a lock is answered with a failure.
//!
//! The socket is the whole access control, exactly as it is for `ssh-agent`: it is
//! created 0600 inside the user's runtime directory, so only processes running as that
//! user can reach it.

use crate::vault::model::{SshKey, Vault};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Where the socket lives: the runtime dir when the session has one, else beside the
/// box's own files. A path the user typed wins over both.
pub fn socket_path(chosen: &str) -> PathBuf {
    if !chosen.trim().is_empty() {
        return PathBuf::from(chosen.trim());
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("pandora-agent.sock");
    }
    crate::vault::store::data_dir().join("agent.sock")
}

/// The protocol's message types, from `draft-miller-ssh-agent`.
const FAILURE: u8 = 5;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;

/// Nothing legitimate is this large; anything that claims to be is hung up on.
const MAX_MESSAGE: usize = 256 * 1024;

/// The keys an agent may answer with: `None` is a locked box, and a locked box says no.
type Keys = Arc<RwLock<Option<Vec<SshKey>>>>;

pub struct Agent {
    socket: PathBuf,
    keys: Keys,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// What the snapshot was made from, so the per-frame check copies nothing.
    seen: Option<(u64, usize, usize)>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            socket: PathBuf::new(),
            keys: Arc::new(RwLock::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            seen: None,
        }
    }
}

impl Agent {
    pub fn running(&self) -> bool {
        self.thread.is_some()
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Bind the socket and start answering. A socket left by a dead process is taken
    /// over: it is ours, and a stale one only means the last run was killed.
    pub fn start(&mut self, socket: PathBuf) -> Result<(), String> {
        if self.running() {
            return Ok(());
        }
        if let Some(dir) = socket.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
        }
        let _ = std::fs::remove_file(&socket);
        let listener =
            UnixListener::bind(&socket).map_err(|e| format!("cannot bind {socket:?}: {e}"))?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot protect {socket:?}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot listen: {e}"))?;

        let keys = self.keys.clone();
        let stop = self.stop.clone();
        stop.store(false, Ordering::SeqCst);
        let path = socket.clone();
        self.thread = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let keys = keys.clone();
                        std::thread::spawn(move || {
                            let _ = serve(stream, &keys);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
            let _ = std::fs::remove_file(&path);
        }));
        self.socket = socket;
        self.seen = None;
        Ok(())
    }

    /// Stop answering and take the socket away. The snapshot is dropped first, so a
    /// request already in flight finds nothing.
    pub fn stop(&mut self) {
        if let Ok(mut keys) = self.keys.write() {
            keys.take();
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if !self.socket.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.socket);
        }
        self.seen = None;
    }

    /// Keep the agent in step with the box: start it, hand it the allowed keys, or stop
    /// it. Called every frame, and cheap when nothing moved.
    pub fn poll(&mut self, wanted: Option<&Path>, vault: Option<&Vault>) {
        let Some(socket) = wanted else {
            if self.running() {
                self.stop();
            }
            return;
        };
        if self.running() && self.socket != socket {
            self.stop();
        }
        if !self.running() && self.start(socket.to_path_buf()).is_err() {
            return;
        }
        let Some(vault) = vault else {
            // Locked: nothing answers until a box is open again.
            if self.seen.is_some() {
                if let Ok(mut keys) = self.keys.write() {
                    keys.take();
                }
                self.seen = None;
            }
            return;
        };
        let allowed = vault.ssh_keys.iter().filter(|it| it.allowed).count();
        let seen = (vault.seq, vault.ssh_keys.len(), allowed);
        if self.seen == Some(seen) {
            return;
        }
        self.seen = Some(seen);
        let list: Vec<SshKey> = vault
            .ssh_keys
            .iter()
            .filter(|it| it.allowed)
            .cloned()
            .collect();
        if let Ok(mut keys) = self.keys.write() {
            *keys = (!list.is_empty()).then_some(list);
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One connection: requests until the client hangs up. A locked box is a failure, and a
/// request nobody understands is one too — never a guess.
fn serve(mut stream: UnixStream, keys: &Keys) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    while let Some(request) = read_message(&mut stream)? {
        let reply = answer(&request, keys);
        write_message(&mut stream, &reply)?;
    }
    Ok(())
}

fn answer(request: &[u8], keys: &Keys) -> Vec<u8> {
    let Some(kind) = request.first() else {
        return vec![FAILURE];
    };
    let list: Vec<SshKey> = match keys.read() {
        // Filtered here as well as when the snapshot is made: the rule is that a key
        // nobody allowed is never offered, and it holds whatever the snapshot holds.
        Ok(guard) => guard
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|it| it.allowed)
            .collect(),
        Err(_) => return vec![FAILURE],
    };
    match (*kind, list.is_empty()) {
        (REQUEST_IDENTITIES, false) => {
            let mut out = vec![IDENTITIES_ANSWER];
            put_u32(&mut out, list.len() as u32);
            for key in &list {
                put_string(&mut out, &public_blob(key));
                put_string(&mut out, key.comment.as_bytes());
            }
            out
        }
        (SIGN_REQUEST, false) => {
            let mut at = 1;
            let Some(wanted) = take_string(request, &mut at) else {
                return vec![FAILURE];
            };
            let Some(data) = take_string(request, &mut at) else {
                return vec![FAILURE];
            };
            let Some(key) = list.iter().find(|it| public_blob(it) == wanted) else {
                return vec![FAILURE];
            };
            match sign(key, data) {
                Some(signature) => {
                    let mut out = vec![SIGN_RESPONSE];
                    put_string(&mut out, &signature);
                    out
                }
                None => vec![FAILURE],
            }
        }
        _ => vec![FAILURE],
    }
}

/// The key as SSH names it: `string "ssh-ed25519"`, then the 32 bytes.
pub fn public_blob(key: &SshKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    put_string(&mut out, b"ssh-ed25519");
    put_string(&mut out, key.public.as_slice());
    out
}

/// Import an unencrypted OpenSSH ed25519 private key. The file is parsed here, the
/// seed is checked against the public half it carries, and nothing is written to
/// disk: the result is a key the box can keep like one it made itself.
pub fn import_openssh(text: &str, name: &str) -> Result<SshKey, String> {
    let body: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("-----"))
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body)
        .map_err(|e| format!("not a base64 OpenSSH key: {e}"))?;

    let magic = b"openssh-key-v1\0";
    if !bytes.starts_with(magic) {
        return Err("not an OpenSSH private key".to_owned());
    }
    let mut at = magic.len();

    let cipher = take_str(&bytes, &mut at)?;
    if cipher != "none" {
        return Err("encrypted OpenSSH keys are not supported".to_owned());
    }
    let kdf = take_str(&bytes, &mut at)?;
    if kdf != "none" {
        return Err("encrypted OpenSSH keys are not supported".to_owned());
    }
    let _options = take_blob(&bytes, &mut at)?;
    let count = take_u32(&bytes, &mut at).ok_or("truncated OpenSSH key")?;
    if count != 1 {
        return Err("expected exactly one key".to_owned());
    }
    let public_blob = take_blob(&bytes, &mut at)?;
    let private_blob = take_blob(&bytes, &mut at)?;

    // Public blob: string "ssh-ed25519", then the 32 public bytes.
    let mut pub_at = 0;
    let key_type = take_str_bytes(&public_blob, &mut pub_at)?;
    if key_type != "ssh-ed25519" {
        return Err("only ed25519 keys can be imported".to_owned());
    }
    let public_bytes = take_blob(&public_blob, &mut pub_at)?;
    if public_bytes.len() != 32 {
        return Err("unexpected public key length".to_owned());
    }
    let public: [u8; 32] = public_bytes.as_slice().try_into().unwrap();

    // Private blob: two check ints, then key type, seed+public, comment, padding.
    let mut priv_at = 0;
    let check1 = take_u32(&private_blob, &mut priv_at).ok_or("truncated OpenSSH key")?;
    let check2 = take_u32(&private_blob, &mut priv_at).ok_or("truncated OpenSSH key")?;
    if check1 != check2 {
        return Err("corrupt OpenSSH key (check ints differ)".to_owned());
    }
    let priv_type = take_str_bytes(&private_blob, &mut priv_at)?;
    if priv_type != "ssh-ed25519" {
        return Err("only ed25519 keys can be imported".to_owned());
    }
    // OpenSSH stores the public key first, then the private half (seed + public).
    let _ = take_blob(&private_blob, &mut priv_at)?;
    let private_half = take_blob(&private_blob, &mut priv_at)?;
    if private_half.len() != 64 {
        return Err("unexpected private key length".to_owned());
    }
    let embedded_public: [u8; 32] = private_half[32..].try_into().unwrap();
    if embedded_public != public {
        return Err("the public half does not match the seed".to_owned());
    }
    let seed_bytes = &private_half[..32];
    let seed: [u8; 32] = seed_bytes.try_into().unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let derived = signing.verifying_key().to_bytes();
    if derived != public {
        return Err("the seed does not derive the public key".to_owned());
    }
    let comment = take_str_bytes(&private_blob, &mut priv_at).unwrap_or_default();

    let now = crate::vault::model::now_secs();
    Ok(SshKey {
        id: uuid::Uuid::new_v4().to_string(),
        name: if name.trim().is_empty() {
            "imported key".to_owned()
        } else {
            name.to_owned()
        },
        private: zeroize::Zeroizing::new(seed.to_vec()),
        public: zeroize::Zeroizing::new(public.to_vec()),
        comment: if comment.is_empty() {
            "imported".to_owned()
        } else {
            comment
        },
        allowed: false,
        created: now,
        modified: now,
    })
}

fn take_str(bytes: &[u8], at: &mut usize) -> Result<String, String> {
    let blob = take_blob(bytes, at)?;
    String::from_utf8(blob).map_err(|_| "invalid utf-8 in OpenSSH key".to_owned())
}

fn take_str_bytes(bytes: &[u8], at: &mut usize) -> Result<String, String> {
    take_str(bytes, at)
}

fn take_blob(bytes: &[u8], at: &mut usize) -> Result<Vec<u8>, String> {
    let len = take_u32(bytes, at).ok_or("truncated OpenSSH key")? as usize;
    let end = at.saturating_add(len);
    let slice = bytes.get(*at..end).ok_or("truncated OpenSSH key")?;
    *at = end;
    Ok(slice.to_vec())
}

/// The line a user pastes into `authorized_keys`.
pub fn public_line(key: &SshKey) -> String {
    let mut line = format!(
        "ssh-ed25519 {}",
        crate::vault::codec::display(key.public.as_slice())
    );
    if !key.comment.is_empty() {
        line.push(' ');
        line.push_str(&key.comment);
    }
    line
}

/// `string "ssh-ed25519"`, then the 64-byte signature.
fn sign(key: &SshKey, data: &[u8]) -> Option<Vec<u8>> {
    let seed: [u8; 32] = key.private.as_slice().try_into().ok()?;
    let signing = SigningKey::from_bytes(&seed);
    let signature = signing.sign(data);
    let mut out = Vec::with_capacity(80);
    put_string(&mut out, b"ssh-ed25519");
    put_string(&mut out, &signature.to_bytes());
    Some(out)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}

fn take_u32(data: &[u8], at: &mut usize) -> Option<u32> {
    let bytes = data.get(*at..*at + 4)?;
    *at += 4;
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

fn take_string<'a>(data: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let len = take_u32(data, at)? as usize;
    let bytes = data.get(*at..*at + len)?;
    *at += len;
    Some(bytes)
}

fn read_message(stream: &mut UnixStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match stream.read_exact(&mut len) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Ok(None);
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_message(stream: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A key's seed leaves memory when the snapshot does; this is the one place a test
/// reads it.
#[cfg(test)]
fn seed_of(key: &SshKey) -> Vec<u8> {
    key.private.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::now_secs;
    use ed25519_dalek::Verifier;
    use std::os::unix::net::UnixStream as Pair;

    fn key(allowed: bool) -> SshKey {
        let mut key = SshKey::make(now_secs(), "laptop", "ana@example");
        key.allowed = allowed;
        key
    }

    fn keys(list: Vec<SshKey>) -> Keys {
        Arc::new(RwLock::new(Some(list)))
    }

    fn request_identities() -> Vec<u8> {
        vec![REQUEST_IDENTITIES]
    }

    fn request_sign(blob: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = vec![SIGN_REQUEST];
        put_string(&mut out, blob);
        put_string(&mut out, data);
        put_u32(&mut out, 0);
        out
    }

    /// Only the allowed key is listed, and the signature it gives is the one a server
    /// would verify with the public line it hands out.
    #[test]
    fn an_allowed_key_is_listed_and_signs() {
        let allowed = key(true);
        let denied = key(false);
        let store = keys(vec![allowed.clone(), denied.clone()]);

        let listed = answer(&request_identities(), &store);
        assert_eq!(listed[0], IDENTITIES_ANSWER);
        let mut at = 1;
        let count = take_u32(&listed, &mut at).unwrap();
        assert_eq!(count, 1, "the denied key is not offered");
        let blob = take_string(&listed, &mut at).unwrap();
        assert_eq!(blob, public_blob(&allowed));
        let comment = take_string(&listed, &mut at).unwrap();
        assert_eq!(comment, b"ana@example");

        let data = b"the challenge a server sends";
        let reply = answer(&request_sign(&public_blob(&allowed), data), &store);
        assert_eq!(reply[0], SIGN_RESPONSE);
        let mut at = 1;
        let signature = take_string(&reply, &mut at).unwrap();
        let mut at = 0;
        let kind = take_string(signature, &mut at).unwrap();
        assert_eq!(kind, b"ssh-ed25519");
        let bytes = take_string(signature, &mut at).unwrap();
        let verifying =
            ed25519_dalek::VerifyingKey::from_bytes(&allowed.public.as_slice().try_into().unwrap())
                .unwrap();
        verifying
            .verify(data, &ed25519_dalek::Signature::from_slice(bytes).unwrap())
            .expect("the server can verify it");
    }

    /// A locked box, a key nobody allowed, and a request that is not understood all
    /// answer with a failure — never with a key.
    #[test]
    fn everything_else_is_a_failure() {
        let allowed = key(true);
        let locked: Keys = Arc::new(RwLock::new(None));
        assert_eq!(answer(&request_identities(), &locked), vec![FAILURE]);
        assert_eq!(
            answer(&request_sign(&public_blob(&allowed), b"x"), &locked),
            vec![FAILURE]
        );

        let store = keys(vec![allowed.clone(), key(false)]);
        let denied = key(false);
        assert_eq!(
            answer(&request_sign(&public_blob(&denied), b"x"), &store),
            vec![FAILURE],
            "a key that is not allowed does not sign"
        );
        assert_eq!(answer(&[99], &store), vec![FAILURE]);
        assert_eq!(answer(&[], &store), vec![FAILURE]);
    }

    /// The whole way round a socket: the protocol's framing, a real connection, and the
    /// key's line as `authorized_keys` would carry it.
    #[test]
    fn the_socket_answers_the_agent_protocol() {
        let key = key(true);
        let store = keys(vec![key.clone()]);
        let (mut client, server) = Pair::pair().unwrap();
        let serving = store.clone();
        let thread = std::thread::spawn(move || serve(server, &serving).unwrap());

        write_message(&mut client, &request_identities()).unwrap();
        let reply = read_message(&mut client).unwrap().unwrap();
        assert_eq!(reply[0], IDENTITIES_ANSWER);

        let data = b"hello";
        write_message(&mut client, &request_sign(&public_blob(&key), data)).unwrap();
        let reply = read_message(&mut client).unwrap().unwrap();
        assert_eq!(reply[0], SIGN_RESPONSE);

        drop(client);
        thread.join().unwrap();

        assert!(public_line(&key).starts_with("ssh-ed25519 "));
        assert!(public_line(&key).ends_with(" ana@example"));
        let seed = crate::vault::codec::display(key.private.as_slice()).to_string();
        assert!(!format!("{key:?}").contains(&seed), "the seed stays hidden");
        assert_eq!(seed_of(&key).len(), 32);
    }

    /// An unencrypted OpenSSH ed25519 key imports into a key the agent can serve,
    /// and its seed derives the public half it carried.
    #[test]
    fn an_openssh_ed25519_key_imports() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB2lScPspZYT0suH+8DoQjoppMzE+kDOCuRHbvWLfEwLAAAAJBxtqOycbaj\nsgAAAAtzc2gtZWQyNTUxOQAAACB2lScPspZYT0suH+8DoQjoppMzE+kDOCuRHbvWLfEwLA\nAAAEDB3av168MoC/QvhMie6OI4JiOfogqbncEURpoFidAxtXaVJw+yllhPSy4f7wOhCOim\nkzMT6QM4K5Edu9Yt8TAsAAAADHRlc3RAcGFuZG9yYQE=\n-----END OPENSSH PRIVATE KEY-----\n";
        let key = import_openssh(pem, "laptop").unwrap();
        assert_eq!(key.name, "laptop");
        assert_eq!(key.private.len(), 32);
        assert_eq!(key.public.len(), 32);
        assert!(!key.allowed);
    }

    /// A socket on disk is 0600 and answers, and stopping takes it away.
    #[test]
    fn a_socket_on_disk_is_private_and_goes_away() {
        let mut agent = Agent::default();
        let dir = std::env::temp_dir().join(format!("pandora-agent-{}", std::process::id()));
        let socket = dir.join("agent.sock");
        agent.start(socket.clone()).unwrap();
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "only the user can reach the agent");
        assert!(agent.running());
        agent.stop();
        assert!(!socket.exists(), "and the socket is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The per-frame poll hands over only what is allowed, and a lock empties it.
    #[test]
    fn the_snapshot_follows_the_box_and_empties_when_it_locks() {
        let mut agent = Agent::default();
        let dir = std::env::temp_dir().join(format!("pandora-agent-poll-{}", std::process::id()));
        let socket = dir.join("agent.sock");

        let mut vault = Vault::new("Box");
        vault.ssh_keys.push(key(true));
        vault.ssh_keys.push(key(false));
        agent.poll(Some(&socket), Some(&vault));
        assert_eq!(agent.keys.read().unwrap().as_ref().unwrap().len(), 1);

        agent.poll(Some(&socket), None);
        assert!(agent.keys.read().unwrap().is_none(), "locked is empty");

        agent.poll(None, Some(&vault));
        assert!(!agent.running(), "and the switch takes the socket away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
