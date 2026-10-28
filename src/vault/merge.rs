//! Two versions of one box folded into one: this device's and the copy a repo or
//! another device wrote. Records meet by id, the newer `modified` wins, and a record
//! both sides changed at nearly the same moment is kept twice instead of one edit
//! being thrown away — a password is not something to lose to a clock.
//!
//! A deletion is a tombstone in `trash`, with the second it happened. It removes the
//! other side's copy unless that copy was edited after it, which is the only reading
//! of "I deleted this" and "I changed this" that loses nothing.

use crate::vault::model::{Entry, Trashed, Vault};
use serde::Serialize;

/// Two edits of one record closer than this are both kept: a clock that disagrees by
/// two minutes is not evidence that one side is stale.
pub const SKEW: u64 = 120;

/// The suffix a second copy wears, so the two are one click apart instead of one lost.
pub const CONFLICT: &str = " (conflict)";

#[derive(Debug, Clone)]
pub struct Merged {
    pub vault: Vault,
    /// Records this side took from the other, because they were newer there.
    pub adopted: usize,
    /// A record both sides changed at once: the other side's copy is kept too.
    pub conflicts: usize,
    /// Live records a tombstone removed.
    pub dropped: usize,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    adopted: usize,
    conflicts: usize,
    dropped: usize,
}

/// Fold `other` into `local`. Neither input is touched; the result is a new box.
pub fn merge(local: &Vault, other: &Vault, now: u64) -> Merged {
    let mut tally = Tally::default();
    let mut vault = Vault::new(&local.name);
    vault.seq = local.seq.max(other.seq);
    // The reader behind a chunked note or file belongs to the box it was opened from;
    // a merged box's own reader is attached by whoever writes it.
    vault.reader = local.reader.clone();

    vault.folders = union(&local.folders, &other.folders, now, &mut tally);
    vault.logins = union(&local.logins, &other.logins, now, &mut tally);
    vault.passkeys = union(&local.passkeys, &other.passkeys, now, &mut tally);
    vault.drive = union(&local.drive, &other.drive, now, &mut tally);
    vault.otps = union(&local.otps, &other.otps, now, &mut tally);

    let stones = tombstones(&local.trash, &other.trash, &mut tally);
    // A tombstone removes what it outlived; what was edited after it stays.
    let mut trash: Vec<Trashed> = Vec::new();
    for stone in stones {
        let id = stone.record.id().to_owned();
        match vault.any(&id).map(|it| it.modified()) {
            Some(modified) if modified > stone.deleted => {}
            Some(_) => {
                vault.remove(&id);
                tally.dropped += 1;
                trash.push(stone);
            }
            None => trash.push(stone),
        }
    }
    vault.trash = trash;
    Merged {
        vault,
        adopted: tally.adopted,
        conflicts: tally.conflicts,
        dropped: tally.dropped,
    }
}

/// One collection, folded by id. A record only one side has is simply there; one both
/// have keeps the newer, and a tie close enough to be a real disagreement is kept
/// twice, the other side's copy wearing [`CONFLICT`].
fn union<T>(local: &[T], other: &[T], now: u64, tally: &mut Tally) -> Vec<T>
where
    T: Mergeable + Serialize + Clone,
{
    let mut out: Vec<T> = local.to_vec();
    for theirs in other {
        let Some(at) = out.iter().position(|ours| ours.id() == theirs.id()) else {
            tally.adopted += 1;
            out.push(theirs.clone());
            continue;
        };
        if same(&out[at], theirs) {
            continue;
        }
        let (ours, theirs_at) = (out[at].modified(), theirs.modified());
        if theirs_at > ours.saturating_add(SKEW) {
            tally.adopted += 1;
            out[at] = theirs.clone();
        } else if ours > theirs_at.saturating_add(SKEW) {
            // Ours is the newer one: nothing to do, and nothing lost.
        } else if theirs_at > ours {
            // Both changed it at once. The newer edit keeps the plain name; the older
            // arrives beside it wearing the suffix, and the user decides which goes.
            tally.adopted += 1;
            tally.conflicts += 1;
            let name = format!("{}{CONFLICT}", out[at].as_name());
            let older = out[at].clone().wearing(&name, now);
            out[at] = theirs.clone();
            out.push(older);
        } else {
            tally.conflicts += 1;
            let name = format!("{}{CONFLICT}", theirs.as_name());
            out.push(theirs.clone().wearing(&name, now));
        }
    }
    out
}

/// Every tombstone once, the later deletion winning.
fn tombstones(local: &[Trashed], other: &[Trashed], tally: &mut Tally) -> Vec<Trashed> {
    let mut out: Vec<Trashed> = local.to_vec();
    for theirs in other {
        let Some(at) = out
            .iter()
            .position(|ours| ours.record.id() == theirs.record.id())
        else {
            tally.adopted += 1;
            out.push(theirs.clone());
            continue;
        };
        if theirs.deleted > out[at].deleted {
            tally.adopted += 1;
            out[at] = theirs.clone();
        }
    }
    out
}

/// Whether two records say the same thing, no matter when they were last touched.
fn same<T: Serialize>(a: &T, b: &T) -> bool {
    fn struck(value: &mut serde_json::Value) {
        if let Some(map) = value.as_object_mut() {
            map.remove("modified");
        }
    }
    let (Ok(mut a), Ok(mut b)) = (serde_json::to_value(a), serde_json::to_value(b)) else {
        return false;
    };
    struck(&mut a);
    struck(&mut b);
    a == b
}

/// What a collection has to be to be folded: its id, when it changed, and a copy of
/// itself under another name.
pub trait Mergeable {
    fn id(&self) -> &str;
    fn modified(&self) -> u64;
    fn as_name(&self) -> String;
    /// A copy with a fresh id, this name, and `now` as its own modification time.
    fn wearing(self, name: &str, now: u64) -> Self;
}

macro_rules! mergeable {
    ($type:ty) => {
        impl Mergeable for $type {
            fn id(&self) -> &str {
                &self.id
            }

            fn modified(&self) -> u64 {
                self.modified
            }

            fn as_name(&self) -> String {
                self.name.clone()
            }

            fn wearing(mut self, name: &str, now: u64) -> Self {
                self.id = uuid::Uuid::new_v4().to_string();
                self.name = name.to_owned();
                self.modified = now;
                self
            }
        }
    };
}

mergeable!(crate::vault::model::Folder);
mergeable!(crate::vault::model::Login);
mergeable!(crate::vault::model::Passkey);
mergeable!(crate::vault::model::Otp);

impl Mergeable for Entry {
    fn id(&self) -> &str {
        Entry::id(self)
    }

    fn modified(&self) -> u64 {
        Entry::modified(self)
    }

    fn as_name(&self) -> String {
        self.name().to_owned()
    }

    fn wearing(mut self, name: &str, now: u64) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        match &mut self {
            Entry::Note(it) => {
                it.id = id;
                it.name = name.to_owned();
                it.modified = now;
            }
            Entry::File(it) => {
                it.id = id;
                it.name = name.to_owned();
                it.modified = now;
            }
        }
        self
    }
}

/// Whether two boxes say the same thing. Neither the generation nor a record's
/// timestamp is part of it: a copy read back after a touch is the same box, and
/// nothing has to be written or sent for a clock.
pub fn equal(a: &Vault, b: &Vault) -> bool {
    fn struck(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.remove("seq");
                map.remove("modified");
                for value in map.values_mut() {
                    struck(value);
                }
            }
            serde_json::Value::Array(list) => {
                for value in list {
                    struck(value);
                }
            }
            _ => {}
        }
    }
    let (Ok(mut a), Ok(mut b)) = (serde_json::to_value(a), serde_json::to_value(b)) else {
        return false;
    };
    struck(&mut a);
    struck(&mut b);
    a == b
}

/// What the status line says after a merge; empty when nothing moved.
pub fn summary(adopted: usize, conflicts: usize) -> String {
    if conflicts > 0 {
        return crate::shared::i18n::tf(
            "merged with the other copy — {conflicts} records kept twice",
            &[("conflicts", &conflicts)],
        );
    }
    if adopted > 0 {
        return crate::shared::i18n::tf(
            "merged with the other copy — {adopted} records came over",
            &[("adopted", &adopted)],
        );
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{Login, Note, Otp};
    use zeroize::Zeroizing;

    fn login(id: &str, name: &str, password: &str, modified: u64) -> Login {
        let mut it = Login::new(1);
        it.id = id.to_owned();
        it.name = name.to_owned();
        it.password = Zeroizing::new(password.to_owned());
        it.modified = modified;
        it
    }

    fn boxed(logins: Vec<Login>) -> Vault {
        let mut v = Vault::new("Box");
        v.logins = logins;
        v
    }

    /// A record only one side has is simply there: an edit made on an offline copy is
    /// not an edit lost.
    #[test]
    fn a_record_only_the_other_side_has_comes_over() {
        let local = boxed(vec![login("a", "A", "one", 10)]);
        let other = boxed(vec![login("b", "B", "two", 20)]);
        let merged = merge(&local, &other, 100);
        assert_eq!(merged.vault.records().count(), 2);
        assert_eq!(merged.adopted, 1);
        assert_eq!(merged.vault.logins[1].password.as_str(), "two");
    }

    /// The newer edit wins, whichever side it is on.
    #[test]
    fn the_newer_edit_wins() {
        let local = boxed(vec![login("a", "A", "old", 100)]);
        let other = boxed(vec![login("a", "A", "new", 900)]);
        let merged = merge(&local, &other, 1_000);
        assert_eq!(merged.vault.logins.len(), 1);
        assert_eq!(merged.vault.logins[0].password.as_str(), "new");
        assert_eq!(merged.adopted, 1);

        let merged = merge(&other, &local, 1_000);
        assert_eq!(
            merged.vault.logins[0].password.as_str(),
            "new",
            "and the same either way round"
        );
        assert_eq!(merged.conflicts, 0);
    }

    /// Two edits a minute apart are a real disagreement, not a stale copy: both stay,
    /// the older one under a name that says so.
    #[test]
    fn two_edits_within_the_skew_are_both_kept() {
        let local = boxed(vec![login("a", "Bank", "mine", 1_000)]);
        let other = boxed(vec![login("a", "Bank", "theirs", 1_030)]);
        let merged = merge(&local, &other, 2_000);
        assert_eq!(
            merged.vault.logins.len(),
            2,
            "neither password is thrown away"
        );
        assert_eq!(merged.conflicts, 1);
        let names: Vec<&str> = merged
            .vault
            .logins
            .iter()
            .map(|it| it.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Bank", "Bank (conflict)"],
            "the newer keeps the name"
        );
        let passwords: Vec<&str> = merged
            .vault
            .logins
            .iter()
            .map(|it| it.password.as_str())
            .collect();
        assert_eq!(passwords, ["theirs", "mine"]);
        assert_ne!(
            merged.vault.logins[0].id, merged.vault.logins[1].id,
            "the copy is a record of its own"
        );
    }

    /// The same record twice, touched at different times but saying the same thing, is
    /// one record: the timestamps are not a disagreement.
    #[test]
    fn a_touch_without_a_change_is_not_a_conflict() {
        let local = boxed(vec![login("a", "Bank", "pw", 100)]);
        let other = boxed(vec![login("a", "Bank", "pw", 130)]);
        let merged = merge(&local, &other, 200);
        assert_eq!(merged.vault.logins.len(), 1);
        assert_eq!(merged.conflicts, 0);
        assert_eq!(merged.vault.logins[0].password.as_str(), "pw");
    }

    /// A deletion is a tombstone with a second on it, and it removes what it outlived.
    #[test]
    fn a_tombstone_removes_the_other_sides_copy() {
        let mut other = boxed(vec![login("a", "Bank", "pw", 50)]);
        other.take("a", 100).unwrap();

        let local = boxed(vec![login("a", "Bank", "pw", 50)]);
        let merged = merge(&local, &other, 200);
        assert!(
            merged.vault.logins.is_empty(),
            "the delete reached this side"
        );
        assert_eq!(merged.dropped, 1);
        assert_eq!(merged.vault.trash.len(), 1, "and the stone is kept");
    }

    /// An edit made after the deletion is newer than the tombstone, and stays.
    #[test]
    fn an_edit_after_the_deletion_survives_it() {
        let mut with_stone = boxed(vec![login("a", "Bank", "pw", 50)]);
        with_stone.take("a", 100).unwrap();

        let local = boxed(vec![login("a", "Bank", "edited later", 300)]);
        let merged = merge(&local, &with_stone, 400);
        assert_eq!(merged.vault.logins.len(), 1, "the later edit is the truth");
        assert_eq!(merged.vault.logins[0].password.as_str(), "edited later");
        assert!(merged.vault.trash.is_empty(), "and the stone is dropped");
    }

    /// The same box read back says the same thing, whatever generation it wears.
    #[test]
    fn a_box_is_the_same_box_whatever_its_generation() {
        let mut a = boxed(vec![login("a", "Bank", "pw", 100)]);
        a.seq = 3;
        let mut b = a.clone();
        b.seq = 9;
        assert!(equal(&a, &b));

        let mut touched = b.clone();
        touched.logins[0].modified = 900;
        assert!(equal(&a, &touched), "a touch is not a change");

        b.logins[0].password = Zeroizing::new("another".to_owned());
        assert!(!equal(&a, &b), "and a changed password is a changed box");
    }

    /// Merging a box with itself changes nothing, so a pull that finds the same
    /// version does not churn the file.
    #[test]
    fn merging_a_box_with_itself_is_a_no_op() {
        let mut local = boxed(vec![login("a", "Bank", "pw", 100)]);
        local.otps.push(Otp::new(1));
        let mut note = Note::new(1);
        note.id = "n".to_owned();
        note.name = "Ideas".to_owned();
        local.drive.push(Entry::Note(note));

        let merged = merge(&local, &local, 500);
        assert_eq!(merged.adopted, 0);
        assert_eq!(merged.conflicts, 0);
        assert_eq!(merged.vault.records().count(), 3);
        assert_eq!(merged.vault.drive.len(), 1);
    }

    /// A note both sides changed within the skew keeps both bodies: a day of writing
    /// is not lost to a two-minute clock difference.
    #[test]
    fn notes_merge_by_id_and_keep_both_bodies() {
        let mut a = Vault::new("Box");
        let mut note = Note::new(1);
        note.id = "n".to_owned();
        note.name = "Ideas".to_owned();
        note.body = Zeroizing::new("one".to_owned());
        note.modified = 100;
        a.drive.push(Entry::Note(note.clone()));

        let mut b = Vault::new("Box");
        note.body = Zeroizing::new("two".to_owned());
        note.modified = 110;
        b.drive.push(Entry::Note(note));

        let merged = merge(&a, &b, 200);
        assert_eq!(merged.vault.drive.len(), 2);
        assert_eq!(merged.conflicts, 1);
        assert_eq!(
            merged
                .vault
                .note_text(note_of(&merged.vault, 0))
                .unwrap()
                .as_str(),
            "two"
        );
        assert_eq!(
            merged
                .vault
                .note_text(note_of(&merged.vault, 1))
                .unwrap()
                .as_str(),
            "one"
        );
    }

    fn note_of(vault: &Vault, at: usize) -> &Note {
        match &vault.drive[at] {
            Entry::Note(it) => it,
            _ => panic!("the drive keeps notes"),
        }
    }
}
