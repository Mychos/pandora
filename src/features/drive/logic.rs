//! Folder tree over the drive view. Folders are storage-only: the plaintext they organise stays inside the same encrypted payload.

use crate::app::{App, Drag, Screen, StatusKind, Tab};
use crate::features::boxes::model::Boxes;
use crate::features::shell::model::Shell;
use crate::shared::i18n::tf;
use crate::shared::utils::csv;
use crate::vault::crypto;
use crate::vault::model::{now_secs, Entry, File, Folder, Kind};
use crate::vault::store;
use crate::vault::tree::below;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The name a document leaves the box under.
///
/// Its own, when the user said so — a download is theirs and a name they gave it
/// is what they will look for. `neutral` instead keeps the extension, so the file
/// still opens with the right app, and replaces everything else with a digest of
/// the entry: the same document lands under the same blank name, and the folder
/// says nothing about what came out of the box.
pub fn export_name(it: &File, neutral: bool) -> String {
    if !neutral {
        return store::sanitize(&it.name);
    }
    let ext = Path::new(&it.name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".to_owned());
    let tag = crypto::digest(it.id.as_bytes());
    format!("pandora-{}.{ext}", &tag[..8])
}

impl Boxes {
    /// Does `id` name a folder of the box on screen? Entries share the id space.
    pub fn is_folder(&self, id: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.vault.folders.iter().any(|f| f.id == id))
    }

    /// Where a new entry should land: the selected folder, else the folder the
    /// Files pane is browsing (`None` on another pane, so the caller picks the root).
    fn target_folder(&self) -> Option<Option<String>> {
        let s = self.session.as_ref()?;
        if let Some(id) = s
            .selected
            .clone()
            .filter(|id| s.vault.folders.iter().any(|f| f.id == *id))
        {
            return Some(Some(id));
        }
        if s.tab == Tab::Drive {
            return Some(s.folder.clone());
        }
        None
    }

    /// The keyboard's way to the ✎ button, which only shows under the pointer or a selected card.
    pub fn edit_folder(&mut self) {
        let Some(id) = self.selected() else {
            return;
        };
        if !self.is_folder(&id) {
            return;
        }
        if let Some(s) = self.session.as_mut() {
            s.editing_folder = Some(id);
        }
    }

    pub fn add_folder(&mut self, parent: Option<String>, name: String) -> Option<String> {
        let s = self.session.as_mut()?;
        let name = name.trim();
        let name = if name.is_empty() { "New folder" } else { name };
        let folder = Folder::new(name, parent, now_secs());
        let id = folder.id.clone();
        s.vault.folders.push(folder);
        s.mark_now();
        Some(id)
    }

    /// Delete a folder; its sub-folders and entries move up to its parent.
    pub fn delete_folder(&mut self, id: &str) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(parent) = s
            .vault
            .folders
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.parent.clone())
        else {
            return;
        };
        s.vault.folders.retain(|f| f.id != id);
        for f in s.vault.folders.iter_mut() {
            if f.parent.as_deref() == Some(id) {
                f.parent = parent.clone();
                f.modified = now_secs();
            }
        }
        for it in s.vault.drive.iter_mut() {
            if it.folder() == Some(id) {
                it.move_to(parent.clone());
                it.touched(now_secs());
            }
        }
        if s.folder.as_deref() == Some(id) {
            s.folder = parent;
        }
        if s.selected.as_deref() == Some(id) {
            s.selected = None;
        }
        if s.editing_folder.as_deref() == Some(id) {
            s.editing_folder = None;
        }
        s.confirm = None;
        s.mark_now();
    }
}

impl Boxes {
    /// Move a folder or entry into `dest` (`None` is the root); a cycle is refused with a message.
    pub fn move_into(&mut self, target: Drag, dest: Option<String>, shell: &mut Shell) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        match target {
            Drag::Folder(id) => {
                if below(&s.vault, &id, dest.as_deref()) {
                    shell.fail("a folder cannot move inside itself");
                    return;
                }
                let Some(f) = s.vault.folders.iter_mut().find(|f| f.id == id) else {
                    return;
                };
                if f.parent == dest {
                    return;
                }
                f.parent = dest;
                f.modified = now_secs();
            }
            Drag::Entry(id) => {
                let now = now_secs();
                let Some(mut record) = s.vault.any_mut(&id) else {
                    return;
                };
                if record.sits_in(dest.as_deref()) {
                    return;
                }
                record.move_to(dest);
                record.touched(now);
            }
        }
        s.mark_now();
        shell.set_status("moved", StatusKind::Saved);
    }

    /// Move `target` to sit just before (`after` false) or after (`after` true)
    /// `anchor`, among its own siblings. A different parent is refused: that is a
    /// move into a folder, not a reorder.
    pub fn reorder(&mut self, target: Drag, anchor: &str, after: bool) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let moved = match target {
            // The id says which list to shift: every pane reorders its own, and a
            // login is at the top level while a drive entry sits in a folder.
            Drag::Entry(id) => match s.vault.any(&id).map(|it| it.kind()) {
                Some(Kind::Login) => shift(&mut s.vault.logins, &id, anchor, after, |l| {
                    (l.id.as_str(), None)
                }),
                Some(Kind::Otp) => shift(&mut s.vault.otps, &id, anchor, after, |o| {
                    (o.id.as_str(), None)
                }),
                Some(Kind::Note | Kind::File) => {
                    shift(&mut s.vault.drive, &id, anchor, after, |n| {
                        (n.id(), n.folder())
                    })
                }
                Some(Kind::Card | Kind::Identity | Kind::Passkey) => false,
                None => false,
            },
            Drag::Folder(id) => shift(&mut s.vault.folders, &id, anchor, after, |f| {
                (f.id.as_str(), f.parent.as_deref())
            }),
        };
        if moved {
            s.mark_now();
        }
    }
}

/// Move the element named `id` next to `anchor`, if both share the same parent
/// (the tuple's second field). Returns whether the list changed.
fn shift<T>(
    list: &mut Vec<T>,
    id: &str,
    anchor: &str,
    after: bool,
    key: impl Fn(&T) -> (&str, Option<&str>),
) -> bool {
    if id == anchor {
        return false;
    }
    let Some(from) = list.iter().position(|x| key(x).0 == id) else {
        return false;
    };
    let Some(parent) = list
        .iter()
        .find(|x| key(x).0 == anchor)
        .map(|x| key(x).1.map(str::to_owned))
    else {
        return false;
    };
    if key(&list[from]).1 != parent.as_deref() {
        return false;
    }
    let item = list.remove(from);
    let at = list.iter().position(|x| key(x).0 == anchor).unwrap();
    list.insert(if after { at + 1 } else { at }, item);
    true
}

impl App {
    /// Attach files dropped on the window; the accessible path is [`Boxes::pick_files`].
    pub fn handle_dropped(&mut self, ctx: &egui::Context) {
        // Only while a vault is on screen: a drop during Unlock or Settings would land on a hidden vault.
        if self.screen != Screen::Main || self.boxes.session.is_none() {
            return;
        }
        let drops: Vec<Arc<dyn egui::DroppedFile + Send + Sync>> =
            ctx.input(|i| i.raw.dropped_files.clone());
        if drops.is_empty() {
            return;
        }
        let mut loaded: Vec<File> = Vec::new();
        let now = now_secs();
        for d in drops {
            let name = file_name(d.path());
            // A drop on the desktop keeps the user's file where it is and only
            // remembers its path; the bytes are read when the box is saved. The
            // web hands over bytes instead, which are already ours.
            if d.path().is_absolute() {
                match fs::metadata(d.path()) {
                    Ok(meta) => loaded.push(File::new_pending(
                        name,
                        d.path().to_path_buf(),
                        meta.len(),
                        false,
                        now,
                    )),
                    Err(e) => self.shell.fail(tf(
                        "could not read {name}: {error}",
                        &[("name", &name), ("error", &e)],
                    )),
                }
            } else {
                match d.bytes() {
                    Ok(bytes) => loaded.push(File::new(name, bytes, now)),
                    Err(e) => self.shell.fail(e.to_string()),
                }
            }
        }
        if loaded.is_empty() {
            return;
        }
        // A drop always lands in the drive, even from another pane: the file
        // is a document, and refusing it on the passwords tab would be noise.
        let destination = self
            .drop_at
            .take()
            .or_else(|| self.boxes.target_folder())
            .unwrap_or_else(|| self.boxes.session.as_ref().and_then(|s| s.folder.clone()));

        // Switching to Files is what makes the upload visible instead of silent.
        let switched = if let Some(s) = self.boxes.session.as_mut() {
            if s.tab != Tab::Drive {
                s.tab = Tab::Drive;
                s.selected = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if switched {
            if let Some(s) = self.boxes.session.as_mut() {
                s.folder = destination.clone();
            }
        }

        self.boxes.add_files(loaded, destination, &mut self.shell);
        // The user asked for it to go now: push the box if it is mirrored.
        self.send(false);
    }
}

impl Boxes {
    /// A phone's picker belongs to the activity, not to a window: it hands back a
    /// copy it made in the app's own storage, which is read here and removed — a
    /// document left in the clear would outlive the box that holds it. Blocks the
    /// frame while the picker is up, exactly as the QR one does.
    #[cfg(target_os = "android")]
    pub fn pick_files(&mut self, shell: &mut Shell) {
        match crate::shared::android::text_with("pick", "*/*") {
            Ok(Some(path)) => self.land_copy(&PathBuf::from(path), shell),
            Ok(None) => {}
            Err(e) => shell.fail(e),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn pick_files(&mut self, shell: &mut Shell) {
        let Some(paths) = rfd::FileDialog::new().pick_files() else {
            return;
        };
        let now = now_secs();
        let mut loaded: Vec<File> = Vec::new();
        for p in paths {
            let name = file_name(&p);
            match fs::metadata(&p) {
                Ok(meta) => loaded.push(File::new_pending(name, p, meta.len(), false, now)),
                Err(e) => shell.fail(tf(
                    "could not read {name}: {error}",
                    &[("name", &name), ("error", &e)],
                )),
            }
        }
        if !loaded.is_empty() {
            self.add_files(loaded, self.target_folder().flatten(), shell);
        }
    }

    /// Read the copy a phone's picker left behind, put it in the drive and drop
    /// the copy. Not `cfg`'d, so the round trip is testable off the phone.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn land_copy(&mut self, path: &std::path::Path, shell: &mut Shell) {
        let name = file_name(path);
        let size = match fs::metadata(path) {
            Ok(meta) => meta.len(),
            Err(e) => {
                let _ = fs::remove_file(path);
                return shell.fail(tf(
                    "could not read {name}: {error}",
                    &[("name", &name), ("error", &e)],
                ));
            }
        };
        // The picker copy is ours: move it out of the picker's temporary spot
        // and keep it until the save has written its chunks. The user's own file
        // is never touched.
        let staging = store::data_dir().join("staging");
        if let Err(e) = fs::create_dir_all(&staging) {
            let _ = fs::remove_file(path);
            return shell.fail(e.to_string());
        }
        let staged = staging.join(format!("pandora-{}.pending", uuid::Uuid::new_v4()));
        let moved = fs::rename(path, &staged)
            .or_else(|_| fs::copy(path, &staged).and_then(|_| fs::remove_file(path)));
        if let Err(e) = moved {
            let _ = fs::remove_file(&staged);
            let _ = fs::remove_file(path);
            return shell.fail(e.to_string());
        }
        self.add_files(
            vec![File::new_pending(name, staged, size, true, now_secs())],
            self.target_folder().flatten(),
            shell,
        );
    }

    fn add_files(&mut self, loaded: Vec<File>, folder: Option<String>, shell: &mut Shell) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let count = loaded.len();
        for mut it in loaded {
            it.folder = folder.clone();
            s.vault.drive.push(Entry::File(it));
        }
        s.mark_now();
        if count == 1 {
            shell.set_status(crate::shared::i18n::t("file added"), StatusKind::Saved);
        } else {
            shell.set_status(
                crate::shared::i18n::tf("{n} files added", &[("n", &count)]),
                StatusKind::Saved,
            );
        }
    }

    /// Export one file to the Downloads folder (*not* re-encrypted — careful!).
    ///
    /// `neutral` is the name the copy goes out under. The prompt asks which one,
    /// because a document called `divorcio.pdf` tells the folder — and the backup
    /// service watching it — everything the box was keeping to itself.
    pub fn export_file(&mut self, it_id: &str, neutral: bool, shell: &mut Shell) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(Entry::File(it)) = s.vault.drive.iter().find(|i| i.id() == it_id) else {
            return;
        };
        let name = export_name(it, neutral);
        let ok = (|| -> Result<PathBuf, String> {
            let dir = csv::download_dir()?;
            let out = dir.join(name);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut file = store::private_sink(&out)?;
            s.vault.copy_file_to(it, &mut file)?;
            file.sync_all()
                .map_err(|e| format!("cannot write {out:?}: {e}"))?;
            Ok(out)
        })();
        match ok {
            Ok(p) => shell.set_status(
                tf("saved to {path}", &[("path", &p.display())]),
                StatusKind::Saved,
            ),
            Err(e) => shell.fail(tf("export failed: {error}", &[("error", &e)])),
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Drag};
    use crate::vault::model::{Login, Note, Otp};

    /// A document leaves under its own name unless the user blanks it: a file
    /// called `Divorcio.pdf` in Downloads tells the folder, the backup service
    /// watching it and whoever opens it everything the box was keeping. The blank
    /// name keeps the extension — the file still has to open with the right app —
    /// and nothing else of the document's.
    #[test]
    fn a_document_leaves_under_its_name_or_under_none() {
        let doc = File::new("Divorcio.pdf".to_owned(), vec![1], 1);
        assert_eq!(export_name(&doc, false), "Divorcio.pdf");

        let blank = export_name(&doc, true);
        assert!(blank.starts_with("pandora-"), "{blank}");
        assert!(blank.ends_with(".pdf"), "{blank}");
        assert!(!blank.contains("Divorcio"), "{blank}");
        assert_eq!(blank.len(), "pandora-".len() + 8 + ".pdf".len());

        // Same document, same blank name; a different one is a different file.
        assert_eq!(export_name(&doc, true), blank);
        let other = File::new("Otra.pdf".to_owned(), vec![1], 1);
        assert_ne!(export_name(&other, true), blank);

        // A name with no extension at all still lands as a file that opens.
        let nameless = File::new("notas".to_owned(), vec![1], 1);
        assert!(export_name(&nameless, true).ends_with(".bin"));
    }

    /// A drag reorders the list the record came from: the id alone says which
    /// one, so a login stays in the passwords pane and a code in the codes pane.
    /// The drive keeps one order for its notes and its files, which is what lets
    /// a document sit wherever a note does.
    #[test]
    fn a_reorder_moves_the_record_in_its_own_collection() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        {
            let s = app.boxes.session.as_mut().unwrap();
            for id in ["a", "b", "c"] {
                let mut login = Login::new(1);
                login.id = format!("l{id}");
                s.vault.logins.push(login);
                let mut note = Note::new(1);
                note.id = format!("n{id}");
                s.vault.drive.push(Entry::Note(note));
                let mut file = File::new(format!("f{id}.bin"), vec![0], 1);
                file.id = format!("f{id}");
                s.vault.drive.push(Entry::File(file));
                let mut otp = Otp::new(1);
                otp.id = format!("o{id}");
                s.vault.otps.push(otp);
            }
        }

        // Each one dragged to the end of its own list.
        for (id, anchor) in [("la", "lc"), ("na", "nc"), ("fa", "fc"), ("oa", "oc")] {
            app.boxes.reorder(Drag::Entry(id.to_owned()), anchor, true);
        }

        let v = &app.boxes.session.as_ref().unwrap().vault;
        assert_eq!(
            v.logins.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            ["lb", "lc", "la"]
        );
        assert_eq!(
            v.drive.iter().map(|e| e.id()).collect::<Vec<_>>(),
            ["nb", "fb", "nc", "na", "fc", "fa"],
            "notes and files shift in the one order they share"
        );
        assert_eq!(
            v.otps.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
            ["ob", "oc", "oa"]
        );
    }

    /// What a phone's picker leaves behind is read, put in the drive and removed:
    /// the copy is the app's own, and a document in the clear would outlive the box.
    #[test]
    fn a_picked_copy_becomes_a_file_and_is_not_left_behind() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        app.boxes.session.as_mut().unwrap().tab = Tab::Drive;
        let p = std::env::temp_dir().join("pandora-picked-copy");
        std::fs::write(&p, b"document").unwrap();

        app.boxes.land_copy(&p, &mut app.shell);

        let v = &app.boxes.session.as_ref().unwrap().vault;
        let Some(Entry::File(file)) = v.drive.first() else {
            panic!("the copy became a file: {:?}", v.drive);
        };
        assert_eq!(file.name, "pandora-picked-copy");
        let pending = file.pending.as_ref().expect("the copy waits for the save");
        assert!(pending.owned, "the staged copy belongs to the app");
        let staged = pending.path.clone();
        assert_eq!(
            v.read_file(file).unwrap().as_slice(),
            b"document",
            "the box can read the pending copy"
        );
        assert!(!p.exists(), "the picker's copy is gone");
        assert!(staged.exists(), "the staged copy waits for the save");

        app.boxes.persist(&mut app.shell);
        assert!(
            !staged.exists(),
            "the staging copy is removed after the save"
        );
    }

    /// A drag reorders within one parent; next to a stranger or itself, nothing moves.
    #[test]
    fn shift_reorders_siblings_and_refuses_the_rest() {
        let mut list: Vec<(String, Option<String>)> = ["a", "b", "c", "d"]
            .iter()
            .map(|id| (id.to_string(), None))
            .collect();
        list[3].1 = Some("x".to_owned());
        fn key((id, parent): &(String, Option<String>)) -> (&str, Option<&str>) {
            (id.as_str(), parent.as_deref())
        }

        assert!(shift(&mut list, "a", "c", true, key), "a moves below c");
        assert_eq!(
            list.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["b", "c", "a", "d"]
        );

        assert!(!shift(&mut list, "b", "d", false, key), "different parent");
        assert!(!shift(&mut list, "c", "c", false, key), "same element");
        assert_eq!(
            list.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["b", "c", "a", "d"],
            "a refused move changes nothing"
        );
    }
}
