use super::header::{header_block, read_header, KdfHeader, V4Header, MAX_P_COST};
use super::write::{finalize, PAD_BLOCK, PAD_FLOOR};
use super::*;
use crate::vault::container::{CHUNK, HEADER_BLOCK};
use crate::vault::matching::{Rule, Uri};
use crate::vault::model::{Entry, File, Note};
use crate::vault::model::{Login, Otp};
use std::env::temp_dir;

fn tmp_path(name: &str) -> PathBuf {
    let p = temp_dir().join(format!(
        "pandora-test-{name}-{}.{EXT}",
        uuid::Uuid::new_v4()
    ));
    let _ = fs::remove_file(&p);
    p
}

#[test]
fn a_save_leaves_a_fresh_stamp_on_the_box() {
    let p = tmp_path("stamp");
    let creds = Credentials::rekey("test", None).unwrap();
    let mut vault = Vault::new("Box");
    save(&p, &mut vault, &creds).unwrap();

    // The stamp a cloud mount hands back: the one the object already had, hours old.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(11 * 60 * 60);
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    let stale = modified_time(&p).unwrap();

    save(&p, &mut vault, &creds).unwrap();

    let fresh = modified_time(&p).unwrap();
    assert!(fresh > stale, "the write moved the stamp forward");
    assert!(
        fresh.elapsed().unwrap() < std::time::Duration::from_secs(5),
        "and it says now, so a client comparing stamps sees the change"
    );
}

/// The one moment a save leaves the box absent is the fallback's delete: a move that
/// fails after it must put the displaced version back, not leave the folder empty.
#[test]
fn a_finish_that_cannot_land_restores_the_seat() {
    let p = tmp_path("finalize");
    fs::write(&p, b"the box before the write").unwrap();
    stash(&p);
    let gone = p.with_extension("tmp-that-is-not-there");

    let err = finalize(&gone, &p).unwrap_err();
    assert!(err.contains("cannot finalize"), "{err}");
    assert_eq!(
        fs::read(&p).unwrap(),
        b"the box before the write",
        "the copy the write displaced came back"
    );
}

#[test]
fn a_stash_keeps_what_a_write_replaces() {
    let p = tmp_path("stash");
    fs::write(&p, b"first").unwrap();
    stash(&p);
    fs::write(&p, b"second").unwrap();
    let backup = stash_path(&p);

    assert_eq!(fs::read(&p).unwrap(), b"second", "the write landed");
    assert_eq!(
        fs::read(&backup).unwrap(),
        b"first",
        "and what it replaced is still readable"
    );

    let missing = tmp_path("stash-none");
    stash(&missing);
    assert!(
        !stash_path(&missing).exists(),
        "nothing to keep, nothing written"
    );

    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(&backup);
}

#[test]
fn the_copy_beside_a_box_can_be_dropped() {
    let p = tmp_path("forget-stash");
    fs::write(&p, b"old").unwrap();
    stash(&p);
    let backup = stash_path(&p);
    assert!(backup.exists(), "the write before the change kept one");

    forget_stash(&p);
    assert!(!backup.exists(), "and the change takes it away");
    assert!(p.exists(), "the box itself is not what this removes");
    forget_stash(&p);

    let _ = fs::remove_file(&p);
}

fn sample_item(id: &str) -> Login {
    let mut it = Login::new(42);
    it.id = id.to_owned();
    it.name = "GitHub".to_owned();
    it.username = "octocat".to_owned();
    it.password = Zeroizing::new("hunter2".to_owned());
    it.url = "https://github.com".to_owned();
    it
}

fn sample_branded_item(id: &str) -> Login {
    let mut it = sample_item(id);
    it.icon = Some("github".to_owned());
    it
}

fn header(path: &Path) -> V4Header {
    read_header(path).unwrap().unwrap()
}

#[test]
fn a_test_build_keeps_its_files_out_of_the_real_data_dir() {
    let ours = data_dir();
    assert!(ours.starts_with(std::env::temp_dir()), "{ours:?}");
    let real = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pandora");
    assert_ne!(
        ours, real,
        "the suite must not write the app's own prefs and sources"
    );
}

#[test]
fn a_source_dir_is_derived_and_stays_inside() {
    let dir = source_dir("github", "octocat/boxes");
    assert_eq!(dir.parent(), Some(data_dir().join("sources").as_path()));
    let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(leaf.len(), 32, "128 bits of digest in hex: {leaf}");
    assert!(leaf.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!leaf.contains("octocat") && !leaf.contains("boxes"));
    assert_ne!(dir, source_dir("github", "octocat/other"));

    let base = data_dir().join("sources");
    let hostile = source_dir("github", "../../etc/passwd");
    assert!(
        !hostile.to_string_lossy().contains(".."),
        "{hostile:?} kept a parent hop"
    );
    assert!(
        hostile.starts_with(&base),
        "{hostile:?} must stay under {base:?}"
    );
}

#[test]
fn only_a_path_inside_sources_is_a_working_copy() {
    let digest = source_digest("github", "octocat/boxes");
    assert!(working_copy(
        &sources_dir().join(&digest).join("work.pandora")
    ));
    assert!(
        !working_copy(&data_dir().join("vault.pandora")),
        "a box the user keeps is not a copy"
    );
    assert!(
        !working_copy(&data_dir().join("sources-old").join("work.pandora")),
        "a folder that only starts with the name is not sources"
    );
}

#[test]
fn a_legacy_source_dir_is_moved_to_its_digest() {
    let target = "octocat/moved-for-the-test";
    let legacy = legacy_source_dir("github", target);
    let _ = fs::remove_dir_all(legacy.parent().unwrap().parent().unwrap());
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("work.pandora"), b"locked").unwrap();

    let dir = source_dir("github", target);
    assert!(dir.join("work.pandora").is_file(), "the copy came along");
    assert!(
        !legacy.exists(),
        "and the folder that named the repo is gone"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_open_roundtrip() {
    let p = tmp_path("roundtrip");
    let (creds, mut vault) = create(&p, "My Vault", "master-pass", None).unwrap();
    assert!(vault.records().next().is_none());
    vault.logins.push(sample_item("abc"));
    save(&p, &mut vault, &creds).unwrap();

    let (creds2, vault2) = open(&p, "master-pass", None).unwrap();
    assert_eq!(vault2.logins[0].id, "abc");
    assert_eq!(vault2.logins[0].password.as_str(), "hunter2");

    let v3 = load(&p, &creds2).unwrap();
    assert_eq!(v3.logins.len(), 1);
    let _ = fs::remove_file(&p);
}

#[test]
fn hostile_kdf_params_are_refused_before_deriving() {
    let p = tmp_path("kdf-bomb");
    let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
    let header = V4Header {
        version: CURRENT_VERSION,
        kdf: KdfHeader {
            algorithm: "argon2id".to_owned(),
            m_cost: u32::MAX,
            t_cost: u32::MAX,
            p_cost: MAX_P_COST * 2,
            salt: codec::encode(&salt),
        },
        box_id: "bomb".to_owned(),
        manifest_offset: HEADER_BLOCK as u64,
        manifest_len: 0,
        manifest_nonce: String::new(),
        recovery: None,
    };
    fs::write(&p, header_block(&header).unwrap()).unwrap();

    assert!(matches!(open(&p, "pw", None), Err(e) if e.contains("kdf params")));
    assert!(!verify(&p, &Zeroizing::new([0u8; crypto::KEY_LEN])));
    let _ = fs::remove_file(&p);
}

#[cfg(unix)]
#[test]
fn a_box_lands_readable_by_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let p = tmp_path("mode");
    let _ = create(&p, "My Vault", "pw", None).unwrap();
    assert_eq!(mode(&p), 0o600, "{p:?} came out readable by anyone");

    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    write_private(&p, b"{}").unwrap();
    assert_eq!(mode(&p), 0o600, "{p:?} kept its old, open mode");
    let _ = fs::remove_file(&p);
}

#[cfg(unix)]
#[test]
fn a_backup_of_a_loose_box_is_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let p = tmp_path("stash-mode");
    fs::write(&p, b"old box").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();

    stash(&p);
    let backup = stash_path(&p);
    assert_eq!(
        mode(&backup),
        0o600,
        "{backup:?} came out readable by anyone"
    );
    assert_eq!(
        fs::read(&backup).unwrap(),
        b"old box",
        "and it holds the box"
    );

    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(&backup);
}

#[test]
fn wrong_password_fails() {
    let p = tmp_path("wrongpass");
    let _ = create(&p, "My Vault", "right", None).unwrap();
    assert!(open(&p, "wrong", None).is_err());
    assert!(open(&p, "", None).is_err());
    let _ = fs::remove_file(&p);
}

#[test]
fn tampered_payload_fails() {
    let p = tmp_path("tamper");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();
    let mut bytes = fs::read(&p).unwrap();
    let header = header(&p);
    let at = (header.manifest_offset + header.manifest_len - 1) as usize;
    bytes[at] ^= 0x20;
    fs::write(&p, &bytes).unwrap();
    assert!(open(&p, "pw", None).is_err());
    let _ = fs::remove_file(&p);
}

/// `algorithm` does not feed the KDF; the header AAD catches the edit.
#[test]
fn tampered_header_fails() {
    let p = tmp_path("hdrtamper");
    let _ = create(&p, "My Vault", "pw", None).unwrap();
    let mut bytes = fs::read(&p).unwrap();
    let at = bytes
        .windows(b"argon2id".len())
        .position(|window| window == b"argon2id")
        .expect("the header names the KDF");
    bytes[at..at + 8].copy_from_slice(b"argon2ii");
    fs::write(&p, &bytes).unwrap();
    assert!(open(&p, "pw", None).is_err());
    let _ = fs::remove_file(&p);
}

#[test]
fn a_legacy_box_is_refused_and_not_overwritten() {
    let p = tmp_path("legacy-refused");
    let legacy = br#"{"version":3,"kdf":{"algorithm":"argon2id","m_cost":65536,"t_cost":3,"p_cost":4,"salt":""},"nonce":"","ciphertext":""}"#.to_vec();
    fs::write(&p, &legacy).unwrap();

    let err = open(&p, "pw", None)
        .err()
        .expect("a legacy box must not open");
    assert!(err.contains("v4"), "{err}");
    assert!(!verify(&p, &Zeroizing::new([0u8; crypto::KEY_LEN])));

    let creds = Credentials::rekey("pw", None).unwrap();
    let mut vault = Vault::new("New");
    let err = save(&p, &mut vault, &creds).unwrap_err();
    assert!(err.contains("refusing to overwrite"), "{err}");
    assert_eq!(fs::read(&p).unwrap(), legacy, "the old box is untouched");
    let _ = fs::remove_file(&p);
}

#[test]
fn payload_is_padded_to_a_block() {
    let p = tmp_path("pad");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();
    let floor = fs::metadata(&p).unwrap().len();
    assert!(
        floor >= PAD_FLOOR as u64,
        "an empty box is never its own few hundred bytes: {floor}"
    );
    assert_eq!(floor % PAD_BLOCK as u64, 0);

    let mut big = sample_item("big");
    big.notes = Zeroizing::new("x".repeat(PAD_FLOOR));
    vault.logins.push(big);
    save(&p, &mut vault, &creds).unwrap();
    let size = fs::metadata(&p).unwrap().len();
    assert_eq!(size % PAD_BLOCK as u64, 0);
    assert!(size > floor, "a bigger box grows: {size} vs {floor}");
    let _ = fs::remove_file(&p);
}

/// A passkey survives the file: its bytes come back as bytes, and the manifest carries
/// no plaintext of the key.
#[test]
fn a_passkey_roundtrips_through_the_box() {
    let p = tmp_path("passkey");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let mut it = crate::vault::model::Passkey::new(1, "example.com");
    it.name = "Example".to_owned();
    it.credential_id = Zeroizing::new(vec![9, 8, 7, 6]);
    it.user_handle = Zeroizing::new(vec![1, 1, 1]);
    it.private_key = Zeroizing::new(b"never-printed".to_vec());
    it.counter = 7;
    vault.passkeys.push(it);
    save(&p, &mut vault, &creds).unwrap();

    let cover = open(&p, "pw", None).unwrap().1;
    assert_eq!(cover.passkeys.len(), 1);
    let saved = &cover.passkeys[0];
    assert_eq!(saved.name, "Example");
    assert_eq!(saved.rp_id, "example.com");
    assert_eq!(saved.credential_id.as_slice(), &[9, 8, 7, 6]);
    assert_eq!(saved.user_handle.as_slice(), &[1, 1, 1]);
    assert_eq!(saved.private_key.as_slice(), b"never-printed");
    assert_eq!((saved.alg, saved.counter), (-7, 7));
    let _ = fs::remove_file(&p);
}

#[test]
fn v4_chunks_files_and_notes() {
    let p = tmp_path("v4chunks");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let payload: Vec<u8> = (0..(2 * CHUNK + 123)).map(|i| (i % 251) as u8).collect();
    let body = "hello ".repeat(30_000);
    let mut note = Note::new(1);
    note.name = "Note".to_owned();
    note.body = Zeroizing::new(body.clone());
    vault.drive.push(Entry::Note(note));
    vault.drive.push(Entry::File(File::new(
        "doc.bin".to_owned(),
        payload.clone(),
        1,
    )));
    save(&p, &mut vault, &creds).unwrap();

    let saved = match &vault.drive[1] {
        Entry::File(file) => file,
        _ => panic!("the drive keeps the file"),
    };
    assert!(saved.record.is_some(), "the file has a record");
    assert!(saved.data.is_empty(), "the inline bytes left memory");
    assert_eq!(saved.bytes(), payload.len());

    let (_c, opened) = open(&p, "pw", None).unwrap();
    let (note, file) = match (&opened.drive[0], &opened.drive[1]) {
        (Entry::Note(note), Entry::File(file)) => (note, file),
        _ => panic!("the drive keeps both entries"),
    };
    assert!(file.record.is_some());
    assert!(file.data.is_empty());
    assert_eq!(
        opened.read_file(file).unwrap().as_slice(),
        payload.as_slice(),
        "the chunks come back byte for byte"
    );
    assert_eq!(opened.note_text(note).unwrap().as_str(), body);
    let _ = fs::remove_file(&p);
}

#[test]
fn a_failed_write_restores_every_staged_entry() {
    let p = tmp_path("rollback");
    let missing = tmp_path("rollback-missing");
    let creds = Credentials::rekey("pass", None).unwrap();
    let mut vault = Vault::new("Box");
    vault.drive.push(Entry::File(File::new(
        "kept.bin".to_owned(),
        vec![7; 100],
        1,
    )));
    let mut note = Note::new(1);
    note.body = Zeroizing::new("kept note".to_owned());
    vault.drive.push(Entry::Note(note));
    vault.drive.push(Entry::File(File::new_pending(
        "gone.bin".to_owned(),
        missing,
        10,
        false,
        1,
    )));

    assert!(
        save(&p, &mut vault, &creds).is_err(),
        "the missing file fails it"
    );
    let Entry::File(file) = &vault.drive[0] else {
        panic!("the file is still there");
    };
    assert_eq!(
        file.data.as_slice(),
        &[7u8; 100],
        "the inline bytes came back"
    );
    assert!(file.record.is_none());
    let Entry::Note(note) = &vault.drive[1] else {
        panic!("the note is still there");
    };
    assert_eq!(note.body.as_str(), "kept note");
    assert!(note.loaded);
    let Entry::File(pending) = &vault.drive[2] else {
        panic!("the pending file is still there");
    };
    assert!(pending.pending.is_some(), "the pending path came back");
    assert!(pending.record.is_none());
    let _ = fs::remove_file(&p);
}

#[test]
fn a_pending_file_is_chunked_on_save() {
    let p = tmp_path("pending");
    let src = temp_dir().join(format!("pandora-src-{}.bin", uuid::Uuid::new_v4()));
    let data = vec![3u8; CHUNK + 17];
    fs::write(&src, &data).unwrap();
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    vault.drive.push(Entry::File(File::new_pending(
        "src.bin".to_owned(),
        src.clone(),
        data.len() as u64,
        true,
        1,
    )));
    save(&p, &mut vault, &creds).unwrap();
    assert!(!src.exists(), "the owned copy is removed after the save");

    let (_c, opened) = open(&p, "pw", None).unwrap();
    let Entry::File(file) = &opened.drive[0] else {
        panic!("the pending file became a file");
    };
    assert_eq!(opened.read_file(file).unwrap().as_slice(), data.as_slice());
    let _ = fs::remove_file(&p);
}

#[test]
fn a_rotated_key_reencrypts_records() {
    let p = tmp_path("rekey-records");
    let (creds, mut vault) = create(&p, "My Vault", "old-pass", None).unwrap();
    let data = vec![9u8; 4096];
    vault.drive.push(Entry::File(File::new(
        "doc.bin".to_owned(),
        data.clone(),
        1,
    )));
    let mut note = Note::new(1);
    note.body = Zeroizing::new("old note".to_owned());
    vault.drive.push(Entry::Note(note));
    save(&p, &mut vault, &creds).unwrap();

    // A loaded note edited before the rotation travels with its edit.
    if let Entry::Note(note) = &mut vault.drive[1] {
        note.body = Zeroizing::new("new note".to_owned());
        note.loaded = true;
    }
    let new = Credentials::rekey("new-pass", None).unwrap();
    save(&p, &mut vault, &new).unwrap();
    assert!(open(&p, "old-pass", None).is_err());

    let (_c, opened) = open(&p, "new-pass", None).unwrap();
    let Entry::File(file) = &opened.drive[0] else {
        panic!("the file survived the rotation");
    };
    assert_eq!(opened.read_file(file).unwrap().as_slice(), data.as_slice());
    let Entry::Note(note) = &opened.drive[1] else {
        panic!("the note survived the rotation");
    };
    assert_eq!(opened.note_text(note).unwrap().as_str(), "new note");
    let _ = fs::remove_file(&p);
}

#[test]
fn a_key_from_earlier_opens_the_box_until_its_password_changes() {
    let p = tmp_path("keyed");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let key = creds.key.clone();

    let (_creds, opened) = open_keyed(&p, key.clone()).unwrap();
    assert_eq!(opened.name, "My Vault");

    let rotated = Credentials::rekey("other-password", None).unwrap();
    save(&p, &mut vault, &rotated).unwrap();
    assert!(
        open_keyed(&p, key).is_err(),
        "the key the reader kept no longer fits this file"
    );
    let (_creds, after) = open_keyed(&p, rotated.key).unwrap();
    assert_eq!(after.name, "My Vault");
    let _ = fs::remove_file(&p);
}

#[test]
fn verify_accepts_only_the_right_key() {
    let p = tmp_path("verify");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();
    assert!(verify(&p, &creds.key));

    let salt = codec::decode(&header(&p).kdf.salt).unwrap();
    let wrong =
        crypto::derive_key(&crypto::keyed("nope", None), &salt, &KdfParams::default()).unwrap();
    assert!(!verify(&p, &wrong));
    let _ = fs::remove_file(&p);
}

#[test]
fn creates_parent_dirs() {
    let p = temp_dir()
        .join("pandora-test-nested")
        .join("a")
        .join("b")
        .join(format!("vault.{EXT}"));
    let _ = fs::remove_dir_all(p.parent().unwrap());
    let (c, mut v) = create(&p, "My Vault", "pw", None).unwrap();
    assert!(p.exists());
    save(&p, &mut v, &c).unwrap();
    let _ = fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn create_keeps_the_name() {
    let p = tmp_path("name");
    let (_c, v) = create(&p, "Work", "pw", None).unwrap();
    assert_eq!(v.name, "Work");
    let (_c2, v2) = open(&p, "pw", None).unwrap();
    assert_eq!(v2.name, "Work");
    let _ = fs::remove_file(&p);
}

#[test]
fn icon_roundtrip() {
    let p = tmp_path("icon");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    vault.logins.push(sample_branded_item("a"));
    save(&p, &mut vault, &creds).unwrap();
    let (_c, v) = open(&p, "pw", None).unwrap();
    assert_eq!(v.logins[0].icon.as_deref(), Some("github"));
    let _ = fs::remove_file(&p);
}

#[test]
fn a_code_roundtrips() {
    let p = tmp_path("totp");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let mut code = Otp::new(1);
    code.name = "GitHub".to_owned();
    code.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
    vault.otps.push(code);
    save(&p, &mut vault, &creds).unwrap();
    let (_creds, opened) = open(&p, "pw", None).unwrap();
    assert_eq!(opened.otps[0].name, "GitHub");
    assert_eq!(opened.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");
    let _ = fs::remove_file(&p);
}

#[test]
fn a_login_without_its_optional_fields_loads() {
    let old = r#"{"id":"a","name":"GitHub","username":"u","password":"p","url":"https://github.com","notes":"","created":1,"modified":1}"#;
    let it: Login = serde_json::from_str(old).unwrap();
    assert_eq!(it.icon, None);
    assert_eq!(it.url_match, Rule::Domain);
    assert!(it.extra_urls.is_empty());

    let round = serde_json::to_string(&it).unwrap();
    for key in ["icon", "url_match", "extra_urls"] {
        assert!(!round.contains(key), "{key} in {round}");
    }
}

#[test]
fn an_empty_box_writes_no_empty_collection() {
    let old = r#"{"version":1,"name":"Old"}"#;
    let v: Vault = serde_json::from_str(old).unwrap();
    assert!(v.records().next().is_none());
    assert!(
        v.folders.is_empty()
            && v.logins.is_empty()
            && v.passkeys.is_empty()
            && v.drive.is_empty()
            && v.otps.is_empty()
            && v.trash.is_empty(),
        "a collection the payload does not carry reads as empty"
    );
    assert_eq!(
        v.seq, 0,
        "a generation an older box never wrote reads as absent"
    );

    let round = serde_json::to_string(&v).unwrap();
    assert_eq!(
        round, old,
        "and it is not written back: absent is not empty"
    );
}

/// A box the previous version wrote, byte for byte: it has to keep opening, and a save
/// has to lift it to the version this one writes.
const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/v4-sample.pandora");
/// A box this version writes, for the same reason one version later.
const FIXTURE_V5: &[u8] = include_bytes!("../../../tests/fixtures/v5-sample.pandora");
const FIXTURE_PASSWORD: &str = "pandora-fixture-password";
const FIXTURE_NOTE: &str = "the note the fixture keeps";

/// A stand-in for the key: an answer that costs no hardware and never blocks.
struct Fake(Vec<u8>);

impl Factor for Fake {
    fn respond(&self, _challenge: &[u8]) -> Result<Vec<u8>, String> {
        Ok(self.0.clone())
    }
}

/// A key that is not there: the open has to say so instead of deriving a box nobody
/// can read.
struct Absent;

impl Factor for Absent {
    fn respond(&self, _challenge: &[u8]) -> Result<Vec<u8>, String> {
        Err("no YubiKey answered".to_owned())
    }
}

/// A factor that writes down how many times it was asked and what it was asked, so the
/// one-touch contract and the challenge itself are both testable without a key.
struct Watched {
    calls: std::cell::Cell<usize>,
    challenge: std::cell::RefCell<Vec<u8>>,
    answer: Vec<u8>,
}

impl Watched {
    fn new(answer: Vec<u8>) -> Self {
        Self {
            calls: std::cell::Cell::new(0),
            challenge: std::cell::RefCell::new(Vec::new()),
            answer,
        }
    }
}

impl Factor for Watched {
    fn respond(&self, challenge: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.set(self.calls.get() + 1);
        self.challenge.replace(challenge.to_vec());
        Ok(self.answer.clone())
    }
}

/// A box with no key opens without touching one; a box enrolled with a key is asked
/// once, with the box's own challenge, and never with a challenge of the caller's.
#[test]
fn a_factor_is_asked_once_and_only_when_the_box_needs_it() {
    let p = tmp_path("watched");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();

    let seen = Watched::new(vec![3; 20]);
    assert!(
        open_with(&p, "pw", None, Some(&seen)).is_ok(),
        "the password alone opens a box that never enrolled a key"
    );
    assert_eq!(
        seen.calls.get(),
        0,
        "and the key is not touched for it, even when it is plugged in"
    );

    // Enrolled, the same answer is the only one that opens it, and one open touches it once.
    let enrolled = Credentials::rekey_with("pw", None, Some(&seen.answer)).unwrap();
    save(&p, &mut vault, &enrolled).unwrap();
    let again = Watched::new(seen.answer.clone());
    assert!(open_with(&p, "pw", None, Some(&again)).is_ok());
    assert_eq!(again.calls.get(), 1, "one open, one touch");
    let box_id = header(&p).box_id;
    assert_eq!(
        again.challenge.borrow().as_slice(),
        crate::shared::yubikey::challenge(&box_id).as_slice(),
        "the challenge is the box's own, derived from the id in its header"
    );
    let _ = fs::remove_file(&p);
}

/// A factor is asked before anything else: a box that cannot reach its key says so
/// instead of looking like a wrong password.
#[test]
fn an_unanswerable_factor_speaks_before_the_password_is_judged() {
    let p = tmp_path("no-factor");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();

    let err = open_with(&p, "not even the password", None, Some(&Absent))
        .err()
        .unwrap();
    assert_eq!(err, "no YubiKey answered", "the factor's own words");
    let _ = fs::remove_file(&p);
}

/// A printed code opens the box without the password, and the slot travels with every
/// save: it is in the header the write makes, not something the session keeps on the
/// side.
#[test]
fn a_recovery_code_opens_the_box_and_survives_a_save() {
    let p = tmp_path("recovery");
    let (mut creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let code = crate::vault::recovery::make_code();
    let box_id = header(&p).box_id;
    set_recovery(&mut creds, &code, &box_id).unwrap();
    save(&p, &mut vault, &creds).unwrap();

    assert!(has_recovery(&p));
    let (_c, opened) = open_with_code(&p, &code).unwrap();
    assert_eq!(opened.name, "My Vault");
    assert_eq!(
        open_with_code(&p, &crate::vault::recovery::make_code())
            .err()
            .unwrap(),
        crate::shared::i18n::t("that code does not open this box")
    );

    // A normal save keeps the slot, and the code still opens what it wrote.
    let mut fresh = open(&p, "pw", None).unwrap().1;
    save(&p, &mut fresh, &creds).unwrap();
    assert!(has_recovery(&p));
    assert!(open_with_code(&p, &code).is_ok());

    // A rekey drops it: the wrapped key belonged to the key being replaced.
    let again = Credentials::rekey("pw-2", None).unwrap();
    save(&p, &mut fresh, &again).unwrap();
    assert!(!has_recovery(&p), "a rotated password drops the code");
    assert!(open_with_code(&p, &code).is_err());

    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(stash_path(&p));
}

/// A box enrolled with a key is unreadable without it, and a box enrolled without one
/// never asks: the challenge and the response enter the derivation in one order.
#[test]
fn a_box_enrolled_with_a_key_needs_that_key() {
    let p = tmp_path("keyfactor");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();

    let key = Fake(vec![7; 20]);
    let enrolled = Credentials::rekey_with("pw", None, Some(&key.0)).unwrap();
    save(&p, &mut vault, &enrolled).unwrap();

    let (_, opened) = open_with(&p, "pw", None, Some(&key)).unwrap();
    assert_eq!(opened.name, "My Vault");
    assert!(
        open(&p, "pw", None).is_err(),
        "the password alone is not the box any more"
    );
    assert_eq!(
        open_with(&p, "pw", None, Some(&Fake(vec![8; 20])))
            .err()
            .unwrap(),
        Failure::Wrong.to_string(),
        "another key's answer is another key"
    );
    assert_eq!(
        open_with(&p, "pw", None, Some(&Absent)).err().unwrap(),
        "no YubiKey answered",
        "a key that is not there says so"
    );
    let _ = fs::remove_file(&p);
}

/// The keyfile and the key are mixed in one order, and both are needed when both were
/// enrolled: a box with the two is not opened by either alone.
#[test]
fn a_keyfile_and_a_key_are_mixed_in_one_order() {
    let p = tmp_path("keyfilefactor");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();
    let keyfile = b"the keyfile";
    let key = Fake(vec![1; 20]);
    let enrolled = Credentials::rekey_with("pw", Some(keyfile), Some(&key.0)).unwrap();
    save(&p, &mut vault, &enrolled).unwrap();

    let (creds, _) = open_with(&p, "pw", Some(keyfile), Some(&key)).unwrap();
    assert_eq!(
        creds.needs_keyfile(),
        Some(true),
        "the session knows the keyfile was part of it"
    );
    assert!(open_with(&p, "pw", Some(keyfile), None).is_err());
    assert!(open_with(&p, "pw", None, Some(&key)).is_err());
    let _ = fs::remove_file(&p);
}

#[test]
fn a_box_written_by_this_version_still_opens() {
    let p = tmp_path("fixture");
    fs::write(&p, FIXTURE).unwrap();

    let h = header(&p);
    assert_eq!(h.version, 4, "the fixture is a v4 box");
    assert_eq!(
        (h.kdf.m_cost, h.kdf.t_cost, h.kdf.p_cost),
        (crypto::M_COST, crypto::T_COST, crypto::P_COST),
        "and it was written with today's KDF"
    );

    let (creds, mut vault) = open(&p, FIXTURE_PASSWORD, None).unwrap();
    assert_eq!(vault.name, "Fixture");
    assert_eq!(vault.logins.len(), 1);
    assert_eq!(vault.otps.len(), 1);
    let note = match &vault.drive[0] {
        Entry::Note(note) => note,
        _ => panic!("the drive keeps the note"),
    };
    assert_eq!(vault.note_text(note).unwrap().as_str(), FIXTURE_NOTE);

    save(&p, &mut vault, &creds).unwrap();
    let (_c, again) = open(&p, FIXTURE_PASSWORD, None).unwrap();
    assert_eq!(again.logins[0].name, "GitHub", "a save keeps what it read");
    assert_eq!(
        header(&p).version,
        CURRENT_VERSION,
        "and a v4 box is written as the version this app writes"
    );
    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(stash_path(&p));
}

/// The version this one writes opens with no complaint, and carries no recovery slot
/// until one is made.
#[test]
fn a_box_written_by_this_version_opens() {
    let p = tmp_path("fixture-v5");
    fs::write(&p, FIXTURE_V5).unwrap();
    let h = header(&p);
    assert_eq!(h.version, CURRENT_VERSION);
    assert!(h.recovery.is_none(), "no code was made for it");

    let (_creds, mut vault) = open(&p, FIXTURE_PASSWORD, None).unwrap();
    assert_eq!(vault.logins[0].name, "GitHub");
    save(&p, &mut vault, &_creds).unwrap();
    assert_eq!(header(&p).version, CURRENT_VERSION);

    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(stash_path(&p));
}

/// Run on purpose when the format is meant to move: it writes the fixture the test
/// above holds this version to. `v4-sample.pandora` stays as it is — it is the box the
/// *previous* version wrote, and the compatibility test.
#[test]
#[ignore = "writes tests/fixtures/v5-sample.pandora"]
fn write_the_v4_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v5-sample.pandora");
    let _ = fs::remove_file(&path);
    let (creds, mut vault) = create(&path, "Fixture", FIXTURE_PASSWORD, None).unwrap();

    let mut login = sample_item("fixture-login");
    login.created = 1;
    login.modified = 1;
    vault.logins.push(login);

    let mut note = Note::new(1);
    note.id = "fixture-note".to_owned();
    note.name = "Note".to_owned();
    note.body = Zeroizing::new(FIXTURE_NOTE.to_owned());
    vault.drive.push(Entry::Note(note));

    let mut otp = Otp::new(1);
    otp.id = "fixture-otp".to_owned();
    otp.name = "Code".to_owned();
    otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
    vault.otps.push(otp);

    save(&path, &mut vault, &creds).unwrap();
    forget_stash(&path);
}

#[test]
fn the_default_rule_and_no_extra_urls_stay_out_of_the_payload() {
    let legacy = r#"{"id":"a","name":"GitHub","username":"u","password":"p","url":"https://github.com","notes":"","created":1,"modified":1}"#;
    let it: Login = serde_json::from_str(legacy).unwrap();
    assert_eq!(it.url_match, Rule::Domain);
    assert!(it.extra_urls.is_empty());
    assert_eq!(it.uris().count(), 1, "the stored URL is the one address");

    let round = serde_json::to_string(&it).unwrap();
    assert!(!round.contains("url_match"), "{round}");
    assert!(!round.contains("extra_urls"), "{round}");
}

#[test]
fn rules_and_extra_urls_survive_a_round_trip() {
    let p = tmp_path("rules");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    let mut it = sample_item("a");
    it.url = "example.com".to_owned();
    it.url_match = Rule::Prefix;
    it.extra_urls = vec![
        Uri {
            url: "login.example.com".to_owned(),
            rule: Rule::Host,
        },
        Uri {
            url: "evil.example.com".to_owned(),
            rule: Rule::Never,
        },
    ];
    vault.logins.push(it);
    save(&p, &mut vault, &creds).unwrap();

    let (_c, v) = open(&p, "pw", None).unwrap();
    assert_eq!(v.logins[0].url_match, Rule::Prefix);
    assert_eq!(v.logins[0].extra_urls.len(), 2);
    assert_eq!(v.logins[0].extra_urls[1].rule, Rule::Never);
    let _ = fs::remove_file(&p);
}

#[test]
fn set_password_reencrypts() {
    let p = tmp_path("rekey");
    let (mut creds, mut vault) = create(&p, "My Vault", "old-pass", None).unwrap();
    vault.logins.push(sample_item("x"));
    save(&p, &mut vault, &creds).unwrap();
    creds = Credentials::rekey("new-pass", None).unwrap();
    save(&p, &mut vault, &creds).unwrap();

    assert!(open(&p, "old-pass", None).is_err());
    let (_c2, v2) = open(&p, "new-pass", None).unwrap();
    assert_eq!(v2.logins[0].name, "GitHub");
    assert_eq!(v2.logins[0].password.as_str(), "hunter2");
    let _ = fs::remove_file(&p);
}

#[test]
fn keyfile_roundtrip() {
    let p = tmp_path("keyfile");
    let kf = b"pretend this is a 4 KiB keyfile";
    let (creds, mut vault) = create(&p, "My Vault", "pw", Some(kf)).unwrap();
    assert_eq!(creds.needs_keyfile(), Some(true));
    vault.logins.push(sample_item("k"));
    save(&p, &mut vault, &creds).unwrap();

    let (back, v) = open(&p, "pw", Some(kf)).unwrap();
    assert_eq!(v.logins[0].name, "GitHub");
    assert_eq!(
        back.needs_keyfile(),
        Some(true),
        "the KDF is what notices, not the file"
    );
    let _ = fs::remove_file(&p);
}

#[test]
fn a_keyfile_box_does_not_open_without_its_keyfile() {
    let p = tmp_path("needs-keyfile");
    let (_, _) = create(&p, "My Vault", "pw", Some(b"keyfile")).unwrap();
    let refused = open(&p, "pw", None).err().expect("no keyfile, no key");
    assert_eq!(
        refused,
        Failure::Wrong.to_string(),
        "the same words a wrong password gets: the box does not say"
    );
    assert!(open(&p, "pw", Some(b"other")).is_err());

    let text = serde_json::to_string(&header(&p)).unwrap();
    assert!(
        !text.contains("keyfile"),
        "nothing in the header says a second factor is needed: {text}"
    );
    let _ = fs::remove_file(&p);
}

#[test]
fn a_stray_keyfile_does_not_lock_a_password_box() {
    let p = tmp_path("stray");
    let _ = create(&p, "My Vault", "pw", None).unwrap();
    let (creds, v) = open(&p, "pw", Some(b"keyfile")).unwrap();
    assert_eq!(v.name, "My Vault");
    assert_eq!(
        creds.needs_keyfile(),
        Some(false),
        "the password alone did it"
    );
    let _ = fs::remove_file(&p);
}

#[test]
fn read_keyfile_rejects_empty_and_huge() {
    let p = tmp_path("kf-read");
    fs::write(&p, b"").unwrap();
    assert!(read_keyfile(&p).unwrap_err().contains("empty"));

    fs::write(&p, vec![7u8; (KEYFILE_MAX + 1) as usize]).unwrap();
    assert!(read_keyfile(&p).unwrap_err().contains("8 MB"));

    let _ = fs::remove_file(&p);
}

#[test]
fn a_copy_older_than_the_box_is_refused() {
    let p = tmp_path("rollback");
    let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
    vault.logins.push(sample_item("github"));
    save(&p, &mut vault, &creds).unwrap();
    let older = fs::read(&p).unwrap();
    vault.logins.push(sample_item("bank"));
    save(&p, &mut vault, &creds).unwrap();
    assert_eq!(vault.seq, 3, "a write is a generation");

    let refused =
        adopt(&p, &older, &creds, Some(vault.seq)).expect_err("a rewound copy is not taken");
    assert_eq!(refused, Failure::RolledBack.to_string());
    assert_eq!(load(&p, &creds).unwrap().seq, 3, "the box is untouched");
    assert!(
        !p.with_extension(format!("{EXT}.candidate")).exists(),
        "and nothing of the refused copy is left beside it"
    );

    // The same bytes are what a restore wants: going back on purpose.
    let restored = adopt(&p, &older, &creds, None).unwrap();
    assert_eq!(restored.seq, 2);
    assert_eq!(restored.logins.len(), 1, "the older box, as it was");

    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(stash_path(&p));
}

#[test]
fn the_header_reveals_no_kdf_flag() {
    for (name, keyfile) in [
        ("flag-plain", None),
        ("flag-keyfile", Some(&b"the keyfile"[..])),
    ] {
        let p = tmp_path(name);
        let _ = create(&p, "My Vault", "pw", keyfile).unwrap();

        let header = header(&p);
        let text = serde_json::to_string(&header).unwrap();
        assert!(
            !text.contains("keyfile"),
            "the header says too much: {text}"
        );

        let kdf = serde_json::to_value(&header.kdf).unwrap();
        let kdf = kdf.as_object().expect("a kdf object");
        let mut fields: Vec<&str> = kdf.keys().map(String::as_str).collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            ["algorithm", "m_cost", "p_cost", "salt", "t_cost"],
            "nothing else rides the header, whatever the box needs"
        );
        let _ = fs::remove_file(&p);
    }
}

#[test]
fn a_copy_that_does_not_open_never_replaces_the_box() {
    let src = tmp_path("adopt-src");
    let dst = tmp_path("adopt-dst");
    let (creds, mut vault) = create(&src, "Repo", "pw", None).unwrap();
    save(&src, &mut vault, &creds).unwrap();
    let bytes = fs::read(&src).unwrap();

    assert_eq!(adopt(&dst, &bytes, &creds, None).unwrap().name, "Repo");

    let (own, _) = create(&dst, "Local", "other", None).unwrap();
    let local = fs::read(&dst).unwrap();
    assert!(adopt(&dst, &bytes, &own, None).is_err(), "another password");
    assert_eq!(fs::read(&dst).unwrap(), local, "the local box is intact");
    assert!(
        !dst.with_extension(format!("{EXT}.candidate")).exists(),
        "and no half-written candidate is left behind"
    );

    let _ = fs::remove_file(&src);
    let _ = fs::remove_file(&dst);
}

/// A box's name becomes the file it is kept in, and a name with nothing in it
/// proposes nothing rather than a `vault.pandora` nobody typed.
#[test]
fn a_box_name_becomes_a_file_name() {
    assert_eq!(file_name("My Box").as_deref(), Some("my-box.pandora"));
    assert_eq!(
        file_name("  Café  de  Ana ").as_deref(),
        Some("café-de-ana.pandora"),
        "a path is UTF-8: the letter the user typed is the letter that is kept"
    );
    assert_eq!(file_name("a--b").as_deref(), Some("a-b.pandora"));
    assert_eq!(
        file_name("100% Ácido").as_deref(),
        Some("100-ácido.pandora")
    );
    assert_eq!(
        file_name("../../etc/passwd").as_deref(),
        Some("etc-passwd.pandora"),
        "no separator survives, so a name cannot leave its folder"
    );
    assert!(file_name("   ").is_none());
    assert!(file_name("...").is_none());
}
