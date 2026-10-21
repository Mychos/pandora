//! End-to-end over the real binary: the framed protocol on its stdin/stdout, the
//! `_id` it echoes back, and the rule that a password only ever leaves through
//! `reveal`.

use pandora::vault::matching::Rule;
use pandora::vault::model::{Entry, Login, Note, Otp};
use pandora::vault::store;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use zeroize::Zeroizing;

struct Host {
    process: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Host {
    /// Launched the way a browser launches it: the caller as our own argv — Chromium
    /// puts the extension origin there, Firefox the add-on id — and whatever else the
    /// test needs in the environment.
    fn spawned(args: &[&str], env: &[(&str, String)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pandora-native-host"));
        command
            .args(args)
            .envs(env.iter().map(|(key, value)| (*key, value.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut process = command.spawn().expect("spawn native host");
        let stdin = process.stdin.take().unwrap();
        let stdout = process.stdout.take().unwrap();
        Self {
            process,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, message: Value) -> Value {
        let body = serde_json::to_vec(&message).unwrap();
        let len = u32::try_from(body.len()).unwrap().to_ne_bytes();
        self.stdin.write_all(&len).unwrap();
        self.stdin.write_all(&body).unwrap();
        self.stdin.flush().unwrap();

        let mut len = [0u8; 4];
        self.stdout.read_exact(&mut len).unwrap();
        let mut buf = vec![0u8; u32::from_ne_bytes(len) as usize];
        self.stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Closing stdin is the clean end of input; the host exits.
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn the_extension_sees_sites_and_reveals_one_password() {
    let path =
        std::env::temp_dir().join(format!("pandora-host-e2e-{}.pandora", uuid::Uuid::new_v4()));
    // Its own data dir, with unlocking from the browser turned on: this test is about
    // the host opening a box itself, which is off by default.
    let dir = std::env::temp_dir().join(format!("pandora-host-data-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("prefs.json"), br#"{"unlock_from_browser":true}"#).unwrap();
    let env = [
        ("PANDORA_DATA_DIR", dir.to_string_lossy().into_owned()),
        (
            "PANDORA_BRIDGE_SOCKET",
            dir.join("none.sock").to_string_lossy().into_owned(),
        ),
    ];
    let (creds, mut vault) = store::create(&path, "My Vault", "master-pass", None).unwrap();
    let mut item = Login::new(1);
    item.name = "GitHub".to_owned();
    item.username = "octocat".to_owned();
    item.password = Zeroizing::new("hunter2".to_owned());
    item.url = "https://github.com/login".to_owned();
    let id = item.id.clone();
    vault.logins.push(item);
    // The same site, pinned to one host: it must sort above the domain rule and
    // name the URL that claimed the page.
    let mut pinned = Login::new(1);
    pinned.name = "GitHub Admin".to_owned();
    pinned.username = "admin".to_owned();
    pinned.password = Zeroizing::new("s3cret".to_owned());
    pinned.url = "login.github.com".to_owned();
    pinned.url_match = Rule::Host;
    let pinned_id = pinned.id.clone();
    vault.logins.push(pinned);
    vault.drive.push(Entry::Note(Note::new(1))); // no URL, never matches
    let mut otp = Otp::new(1);
    otp.name = "GitHub".to_owned();
    otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
    let otp_id = otp.id.clone();
    vault.otps.push(otp);
    store::save(&path, &mut vault, &creds).unwrap();
    let path_str = path.to_string_lossy().into_owned();

    let mut host = Host::spawned(&[], &env);

    // Locked: no entries, and the pairing id comes back.
    let reply = host.call(json!({ "_id": 1, "type": "entries" }));
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["_id"], 1);

    // A wrong password is refused, a right one unlocks.
    let reply =
        host.call(json!({ "_id": 2, "type": "unlock", "path": path_str, "password": "nope" }));
    assert_eq!(reply["type"], "error");
    let reply = host
        .call(json!({ "_id": 3, "type": "unlock", "path": path_str, "password": "master-pass" }));
    assert_eq!(reply["type"], "unlocked");
    assert_eq!(
        reply["count"], 2,
        "the extension counts the logins it can see"
    );

    // The first version of the extension sent only `host`; it still answers.
    let reply = host.call(json!({ "_id": 4, "type": "match", "host": "gist.github.com" }));
    assert_eq!(reply["type"], "entries");
    let entries = reply["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "the pinned host does not cover a sibling");
    assert_eq!(entries[0]["username"], "octocat");
    assert_eq!(entries[0]["uri"], "https://github.com/login");
    assert!(
        entries[0].get("password").is_none(),
        "a match never carries a password"
    );

    // A full URL: most specific first, each naming the URL that matched it.
    let reply =
        host.call(json!({ "_id": 5, "type": "match", "url": "https://login.github.com/session" }));
    let entries = reply["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0]["username"], "admin",
        "the host rule ranks highest"
    );
    assert_eq!(entries[0]["uri"], "login.github.com");
    assert_eq!(entries[1]["username"], "octocat");

    // Only reveal hands one over, and only for the id asked for.
    let reply = host.call(json!({ "_id": 6, "type": "reveal", "id": id }));
    assert_eq!(reply["type"], "secret");
    assert_eq!(reply["password"], "hunter2");
    let reply = host.call(json!({ "_id": 7, "type": "reveal", "id": pinned_id }));
    assert_eq!(reply["password"], "s3cret");

    // The panel saves a login the box had never heard of.
    let reply = host.call(json!({
        "_id": 8, "type": "create",
        "name": "Example", "username": "eve",
        "password": "typed-in", "url": "https://example.com/login"
    }));
    assert_eq!(reply["type"], "saved");
    assert!(
        reply["entry"].get("password").is_none(),
        "a save hands back the entry, not the secret"
    );
    let saved_id = reply["entry"]["id"].as_str().unwrap().to_owned();
    let reply =
        host.call(json!({ "_id": 9, "type": "match", "url": "https://example.com/session" }));
    assert_eq!(reply["entries"].as_array().unwrap().len(), 1);

    // A rotation keeps the username it was not sent.
    let reply =
        host.call(json!({ "_id": 10, "type": "update", "id": saved_id, "password": "rotated" }));
    assert_eq!(reply["type"], "saved");
    assert_eq!(reply["entry"]["username"], "eve");
    let reply = host.call(json!({ "_id": 11, "type": "reveal", "id": saved_id }));
    assert_eq!(reply["password"], "rotated");

    // A generated password is the length asked for, and is not stored.
    let reply = host.call(json!({ "_id": 12, "type": "generate", "length": 32 }));
    assert_eq!(reply["password"].as_str().unwrap().chars().count(), 32);
    let reply = host.call(json!({ "_id": 13, "type": "generate" }));
    assert_eq!(
        reply["password"].as_str().unwrap().chars().count(),
        20,
        "the recipe's default"
    );

    // The write landed in the file, not just in memory.
    let (_c, on_disk) = store::open(&path, "master-pass", None).unwrap();
    assert!(on_disk.logins.iter().any(|item| item.id == saved_id));

    // The codes: the listing names them and carries neither a seed nor a value, and
    // the value comes one id at a time, derived and about to expire.
    let reply = host.call(json!({ "_id": 16, "type": "codes" }));
    assert_eq!(reply["type"], "codes");
    let codes = reply["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 1);
    assert_eq!(codes[0]["name"], "GitHub");
    let listed = serde_json::to_string(&reply).unwrap();
    assert!(
        !listed.contains("JBSWY3DPEHPK3PXP"),
        "a listing never carries a seed: {listed}"
    );

    let reply = host.call(json!({ "_id": 17, "type": "code", "id": otp_id }));
    assert_eq!(reply["type"], "code");
    let code = reply["code"].as_str().unwrap();
    assert_eq!(code.len(), 6, "a six-digit code by default: {code}");
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    let left = reply["remaining"].as_u64().unwrap();
    assert!((1..=30).contains(&left), "{left}");

    let reply = host.call(json!({ "_id": 18, "type": "code", "id": "no such seed" }));
    assert_eq!(reply["type"], "error");

    // Lock drops it again.
    let reply = host.call(json!({ "_id": 14, "type": "lock" }));
    assert_eq!(reply["type"], "ok");
    let reply = host.call(json!({ "_id": 15, "type": "entries" }));
    assert_eq!(reply["type"], "error");

    drop(host);
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(dir);
}

/// The app is the source of truth: when it is listening, the host hands the request
/// over instead of opening the file, which is what stops the extension asking for the
/// master password.
///
/// The stand-in for the app here is deliberately not the app — it is a socket plus the
/// shared reader — because what is under test is the host's half of the bridge: that
/// it relays, that it carries the browser's word about the caller, and that the
/// requests which are *ours* never leave.
#[cfg(unix)]
#[test]
fn the_host_hands_the_request_to_the_app_when_it_is_listening() {
    use pandora::bridge::protocol;
    use pandora::bridge::serve::{self, Peer};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    let dir = std::env::temp_dir().join(format!("pandora-bridge-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("bridge.sock");
    let boxed = dir.join("relayed.pandora");

    // The box the app has open. The host is never told its password, so if it answers
    // out of this vault it can only have been relayed.
    let (creds, mut vault) = store::create(&boxed, "Relayed", "never-typed", None).unwrap();
    let mut item = Login::new(1);
    item.name = "Relayed".to_owned();
    item.username = "from-the-app".to_owned();
    item.password = Zeroizing::new("apple".to_owned());
    item.url = "example.com".to_owned();
    vault.logins.push(item);
    store::save(&boxed, &mut vault, &creds).unwrap();

    let listener = UnixListener::bind(&socket).unwrap();
    let (seen_tx, seen_rx) = mpsc::channel();
    let app = std::thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let mut stream = stream.unwrap();
            let bytes = protocol::read(&mut stream).unwrap().unwrap();
            let envelope = protocol::parse(&bytes);
            seen_tx.send(envelope.caller.clone()).unwrap();
            let reply = serve::read(&envelope.request.unwrap(), Peer::Open(&vault)).unwrap();
            protocol::reply(&mut stream, envelope.id, &reply).unwrap();
        }
    });

    // Chromium's own convention: the caller's origin as the host's first argument.
    let mut host = Host::spawned(
        &["chrome-extension://faked/"],
        &[(
            "PANDORA_BRIDGE_SOCKET",
            socket.to_string_lossy().into_owned(),
        )],
    );

    let reply = host.call(json!({ "_id": 1, "type": "match", "url": "https://example.com/x" }));
    assert_eq!(reply["type"], "entries");
    assert_eq!(reply["entries"][0]["username"], "from-the-app");
    assert_eq!(reply["entries"][0]["uri"], "example.com");

    // `status` too: the app's box is the one it answers for.
    let reply = host.call(json!({ "_id": 2, "type": "status" }));
    assert_eq!(reply["unlocked"], true);
    assert_eq!(reply["name"], "Relayed");

    // Opening a box is the host's own, so it stays here. It is also off by default
    // now, which is what the message says.
    let reply = host.call(json!({
        "_id": 3, "type": "unlock",
        "path": boxed.to_string_lossy(), "password": "wrong"
    }));
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["message"], "unlock this box in the Pandora app");

    let seen: Vec<_> = seen_rx.try_iter().collect();
    assert_eq!(seen.len(), 2, "unlock is ours and must never leave");
    assert_eq!(
        seen[0].as_deref(),
        Some("chrome-extension://faked/"),
        "the browser's word about the caller rides along"
    );

    app.join().unwrap();
    drop(host);
    let _ = std::fs::remove_dir_all(dir);
}

/// An app that takes the connection and then fails is not the same as no app at all:
/// the host says so, instead of answering out of its own (locked) box as if the app
/// had never been there.
#[cfg(unix)]
#[test]
fn an_app_that_fails_is_reported_and_not_answered_in_its_place() {
    use pandora::bridge::protocol;
    use std::os::unix::net::UnixListener;

    let dir = std::env::temp_dir().join(format!("pandora-bridge-bad-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("bridge.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let app = std::thread::spawn(move || {
        for (index, stream) in listener.incoming().take(2).enumerate() {
            let mut stream = stream.unwrap();
            protocol::read(&mut stream).unwrap().unwrap();
            // The first caller gets an answer that is not one; the second is left
            // to close without answering at all.
            if index == 0 {
                protocol::write(&mut stream, b"not a response").unwrap();
            }
        }
    });

    let mut host = Host::spawned(
        &[],
        &[
            ("PANDORA_DATA_DIR", dir.to_string_lossy().into_owned()),
            (
                "PANDORA_BRIDGE_SOCKET",
                socket.to_string_lossy().into_owned(),
            ),
        ],
    );
    let reply = host.call(json!({ "_id": 1, "type": "status" }));
    assert_eq!(reply["type"], "error");
    assert_eq!(
        reply["message"],
        "the app answered with something Pandora does not understand"
    );

    let reply = host.call(json!({ "_id": 2, "type": "status" }));
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["message"], "the app did not answer");

    app.join().unwrap();
    drop(host);
    let _ = std::fs::remove_dir_all(dir);
}

/// Opening a box from the browser is off unless the user turns it on. That is the one
/// thing that would put a master password inside a browser, and with the app open
/// there is nothing to ask for — so the default is to refuse and say where to open it.
#[test]
fn opening_a_box_from_the_browser_is_off_until_the_user_says_so() {
    let dir = std::env::temp_dir().join(format!("pandora-gate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let boxed = dir.join("gated.pandora");
    store::create(&boxed, "Gated", "right", None).unwrap();

    // Nothing to relay to, so the host is on its own — and its own is not allowed.
    let env = [
        ("PANDORA_DATA_DIR", dir.to_string_lossy().into_owned()),
        (
            "PANDORA_BRIDGE_SOCKET",
            dir.join("none.sock").to_string_lossy().into_owned(),
        ),
    ];

    let mut host = Host::spawned(&[], &env);
    let reply = host.call(json!({ "_id": 1, "type": "status" }));
    assert_eq!(reply["unlocked"], false);
    assert_eq!(
        reply["unlockable"], false,
        "the extension is told not to offer a password field"
    );
    let reply = host.call(json!({
        "_id": 2, "type": "unlock",
        "path": boxed.to_string_lossy(), "password": "right"
    }));
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["message"], "unlock this box in the Pandora app");
    drop(host);

    // The user says yes. Now the host opens it, the way it always used to.
    std::fs::write(dir.join("prefs.json"), br#"{"unlock_from_browser":true}"#).unwrap();
    let mut host = Host::spawned(&[], &env);
    let reply = host.call(json!({ "_id": 3, "type": "status" }));
    assert_eq!(reply["unlockable"], true);
    let reply = host.call(json!({
        "_id": 4, "type": "unlock",
        "path": boxed.to_string_lossy(), "password": "right"
    }));
    assert_eq!(reply["type"], "unlocked");
    assert_eq!(reply["name"], "Gated");

    drop(host);
    let _ = std::fs::remove_dir_all(dir);
}
