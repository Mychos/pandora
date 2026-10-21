use super::header::{header_block, read_header, KdfHeader, V4Header, MAX_P_COST};
use super::write::{PAD_BLOCK, PAD_FLOOR};
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
fn a_stash_keeps_what_a_write_replaces() {
    let p = tmp_path("stash");
    fs::write(&p, b"first").unwrap();
    stash(&p);
    fs::write(&p, b"second").unwrap();
    let backup = p.with_extension(format!("{EXT}.bak"));

    assert_eq!(fs::read(&p).unwrap(), b"second", "the write landed");
    assert_eq!(
        fs::read(&backup).unwrap(),
        b"first",
        "and what it replaced is still readable"
    );

    let missing = tmp_path("stash-none");
    stash(&missing);
    assert!(
        !missing.with_extension(format!("{EXT}.bak")).exists(),
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
    let backup = p.with_extension(format!("{EXT}.bak"));
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
    let backup = p.with_extension(format!("{EXT}.bak"));
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
    let wrong = crypto::derive_key(
        &crypto::material("nope", None),
        &salt,
        &KdfParams::default(),
    )
    .unwrap();
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
    assert!(v.folders.is_empty());

    let round = serde_json::to_string(&v).unwrap();
    for key in ["folders", "logins", "drive", "otps", "trash"] {
        assert!(!round.contains(key), "{key} in {round}");
    }
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
    let _ = fs::remove_file(p.with_extension(format!("{EXT}.bak")));
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
