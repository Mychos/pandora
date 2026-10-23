//! One file over a folder: a write lands in place, a stamp guards it, and a
//! missing file is not a failure. The transport is settled here, on a temp
//! directory, before any of it reaches a phone.

use pandora::features::sync::folder::local::Local;
use pandora::features::sync::folder::{Folder, Guard, Write, NAME};
use std::fs;

/// A write lands in place, and the next one is refused when the file moved since
/// the caller last looked. It lands once the caller passes the stamp it saw.
#[test]
fn a_write_is_guarded_by_the_stamp_it_saw() {
    let shared = tempfile::tempdir().expect("a temp dir");
    let a = Local::new(shared.path());

    assert!(matches!(
        a.write(NAME, b"one", Guard::Missing).expect("written"),
        Write::Filed
    ));
    assert_eq!(a.read(NAME).expect("read"), Some(b"one".to_vec()));
    assert!(
        matches!(
            a.write(NAME, b"zero", Guard::Missing).expect("checked"),
            Write::Moved
        ),
        "a file that is there refuses a write that expected none"
    );

    assert!(
        matches!(
            a.write(NAME, b"two", Guard::Stamp(1)).expect("checked"),
            Write::Moved
        ),
        "a stamp that is not the file's refuses the write"
    );
    assert_eq!(
        a.read(NAME).expect("read"),
        Some(b"one".to_vec()),
        "the refused write did not land"
    );

    let stamp = a.stamp(NAME).expect("stamped").expect("a file");
    assert!(matches!(
        a.write(NAME, b"two", Guard::Stamp(stamp)).expect("written"),
        Write::Filed
    ));
    assert_eq!(a.read(NAME).expect("read"), Some(b"two".to_vec()));

    assert!(matches!(
        a.write(NAME, b"three", Guard::Unguarded).expect("written"),
        Write::Filed
    ));
    assert_eq!(
        a.read(NAME).expect("read"),
        Some(b"three".to_vec()),
        "a provider without stamps writes over the file it just read"
    );

    let stamp = a.stamp(NAME).expect("stamped").expect("a file");
    assert!(matches!(
        a.write(NAME, b"x", Guard::Stamp(stamp)).expect("written"),
        Write::Filed
    ));
    assert_eq!(
        a.read(NAME).expect("read"),
        Some(b"x".to_vec()),
        "a shorter write does not leave the old tail behind"
    );
}

/// A missing file reads as none and is created by a write that expected none.
/// Once it is gone, a guarded write does not silently bring it back.
#[test]
fn a_missing_file_is_created_only_by_a_write_that_expected_none() {
    let shared = tempfile::tempdir().expect("a temp dir");
    let a = Local::new(shared.path());

    assert_eq!(a.read(NAME).expect("read"), None);
    assert_eq!(a.stamp(NAME).expect("stamped"), None);
    assert!(matches!(
        a.write(NAME, b"one", Guard::Missing).expect("written"),
        Write::Filed
    ));

    let stamp = a.stamp(NAME).expect("stamped").expect("a file");
    fs::remove_file(shared.path().join(NAME)).expect("removed");
    assert!(
        matches!(
            a.write(NAME, b"two", Guard::Stamp(stamp)).expect("checked"),
            Write::Moved
        ),
        "a file that disappeared is a move, not a blank page"
    );
    assert_eq!(a.read(NAME).expect("read"), None);
}

/// The folder lists by name and drops by name; a drop that finds nothing is the
/// state it asked for.
#[test]
fn the_folder_lists_and_drops_names() {
    let shared = tempfile::tempdir().expect("a temp dir");
    let a = Local::new(shared.path());
    fs::write(shared.path().join("someone-else.txt"), b"junk").expect("junk");
    a.write(NAME, b"one", Guard::Missing).expect("written");

    let mut names = a.list().expect("listed");
    names.sort();
    assert_eq!(names, vec!["someone-else.txt".to_owned(), NAME.to_owned()]);

    a.remove(NAME).expect("dropped");
    a.remove(NAME).expect("already gone");
    assert_eq!(a.read(NAME).expect("read"), None);
}

/// A temp directory is writable, and the folder says so without touching a file.
#[test]
fn a_temp_folder_is_writable() {
    let shared = tempfile::tempdir().expect("a temp dir");
    let a = Local::new(shared.path());
    assert!(a.writable().expect("asked"));
}
