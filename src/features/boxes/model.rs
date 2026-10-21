//! The boxes the app has open: the one on screen plus the strip behind it, and
//! the panes of the main screen.

use crate::app::Field;
use crate::features::entries::model::{Axis, View};
use crate::features::settings::model::Remote;
use crate::features::shell::confirm::Confirm;
use crate::features::sync::github::Fingerprint;
use crate::vault::model::Vault;
use crate::vault::store;
use crate::vault::store::Credentials;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Passwords,
    Drive,
    Codes,
    Settings,
}

impl Tab {
    /// The pane a record lives in: a restore, a search hit or a click on a
    /// record all land here, so the map lives with the pane names.
    pub fn of(kind: crate::vault::model::Kind) -> Tab {
        use crate::vault::model::Kind;
        match kind {
            Kind::Login => Tab::Passwords,
            Kind::Note | Kind::File => Tab::Drive,
            Kind::Otp => Tab::Codes,
        }
    }
}

#[derive(Default)]
pub struct Boxes {
    pub session: Option<Session>,
    /// The other unlocked vaults, in strip order. The one on screen is `session`,
    /// at position `active` among the tabs.
    pub tabs: Vec<Session>,
    pub active: usize,
    /// When the on-disk box was last checked. The check is a filesystem stat and
    /// has no business running on every frame of a resize gesture.
    pub(crate) watch: Option<std::time::Instant>,
}

/// A folder being named in the create modal: its parent is known, the name is
/// not yet, and the vault knows nothing of it until the user confirms.
#[derive(Clone)]
pub struct NewFolder {
    pub parent: Option<String>,
    pub name: String,
}

pub struct Session {
    pub path: PathBuf,
    pub creds: Credentials,
    pub vault: Vault,
    /// The tab is parked: `vault` is an empty placeholder that keeps the box's
    /// name, and the real payload stays on disk. `parked_seq` is the generation
    /// it had when it was parked, so a pull still compares against the right one.
    pub parked: bool,
    pub parked_seq: u64,
    pub dirty: bool,
    pub last_edit: Option<Instant>,
    pub last_mtime: Option<SystemTime>,
    pub conflict: bool,
    pub ignore_mtime: Option<SystemTime>,
    /// Where this box is mirrored; the local file is the source, the repo a copy.
    pub remote: Option<Remote>,
    /// That the repo already holds this box, so nothing is waiting to go out.
    // ponytail: in memory, so a run starts unsynced. It records *that* a version
    // was sent and never what it held: the commit subject is plaintext in the
    // repo, so nothing of the box's shape may reach it.
    pub baseline: Option<Fingerprint>,
    /// When this box may next be pushed on its own; a run of edits lands as one
    /// commit once they stop. None means nothing is waiting to go out.
    pub push_at: Option<Instant>,
    /// The repo moved and this box has not caught up; a stale sha would keep a
    /// push failing forever, so this outlives a status message.
    pub behind: bool,
    pub selected: Option<String>,
    pub search: String,
    /// The id of the last record deleted in this session, for Ctrl+Z. Only the
    /// last one: the trash is where anything older waits.
    pub last_delete: Option<String>,
    /// The trash window is open. It belongs to the box, like the confirm.
    pub show_trash: bool,
    pub reveal: HashSet<(String, Field)>,
    pub show_new: bool,
    /// The ways a picture can reach the codes pane, open as a modal.
    pub show_qr: bool,
    /// The live scanner while a camera is open for it. None on a desktop, where
    /// nothing is worth pointing at.
    pub scan: Option<crate::features::codes::model::Scan>,
    /// The delete waiting for a yes — an entry or a folder; only one at a time.
    pub confirm: Option<Confirm>,
    pub tab: Tab,
    /// Folder shown by the drive view. None is the drive root.
    pub folder: Option<String>,
    /// Folder ids whose children are hidden in the tree. Also carries the fold of
    /// the passwords pane's generated folders, under their own prefixed keys.
    pub collapsed: HashSet<String>,
    /// How the passwords pane draws the box: a flat list or generated folders.
    pub view: View,
    /// The axis the folders view builds its tree from.
    pub axis: Axis,
    /// The generated folder the folders view is inside, one axis segment per
    /// level. Empty is its root.
    pub at: Vec<String>,
    /// The folder whose rename/delete modal is open, apart from `selected`: a
    /// click only selects a folder.
    pub editing_folder: Option<String>,
    /// The folder the create modal is naming, held outside the vault so opening
    /// it writes nothing: only the confirm puts it in the box.
    pub new_folder: Option<NewFolder>,
    /// The drive entry whose editor is open, apart from `selected`: a click only
    /// selects, opening is a separate gesture. Only the Files pane touches this.
    pub editing_entry: Option<String>,
    /// The drive entry waiting for a destination folder: its "Move to…" button
    /// opened the move modal. Only the Files pane touches this.
    pub moving_entry: Option<String>,
}

impl Boxes {
    pub fn selected(&self) -> Option<String> {
        self.session.as_ref().and_then(|s| s.selected.clone())
    }
}

impl Session {
    /// A session for a box just opened: nothing selected, nothing edited, and the
    /// panes on their defaults. A caller that opened onto particular preferences
    /// overrides them after.
    pub fn fresh(path: PathBuf, creds: Credentials, vault: Vault) -> Self {
        Self {
            path,
            creds,
            vault,
            parked: false,
            parked_seq: 0,
            dirty: false,
            last_edit: None,
            last_mtime: None,
            conflict: false,
            ignore_mtime: None,
            remote: None,
            baseline: None,
            push_at: None,
            behind: false,
            selected: None,
            search: String::new(),
            last_delete: None,
            show_trash: false,
            reveal: HashSet::new(),
            show_new: false,
            show_qr: false,
            scan: None,
            confirm: None,
            tab: Tab::Passwords,
            folder: None,
            collapsed: HashSet::new(),
            view: View::List,
            axis: Axis::Target,
            at: Vec::new(),
            editing_folder: None,
            new_folder: None,
            editing_entry: None,
            moving_entry: None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_edit = Some(Instant::now());
        // The file is written on a 900 ms debounce; the commit waits for the box
        // to settle, because GitHub counts commits against an hourly budget and
        // typing is not a change.
        self.push_at = Some(Instant::now() + crate::features::sync::github::PUSH_QUIET);
    }

    /// Whether a secret field is being shown in the clear. The reveal belongs to
    /// the box, so it goes with the session it was made in.
    pub fn shown(&self, id: &str, field: Field) -> bool {
        self.reveal.contains(&(id.to_owned(), field))
    }

    /// Open or close one of a record's secret fields.
    pub fn show(&mut self, id: &str, field: Field, on: bool) {
        let key = (id.to_owned(), field);
        if on {
            self.reveal.insert(key);
        } else {
            self.reveal.remove(&key);
        }
    }

    /// Park a clean tab: keep the key and the path, drop the decrypted payload.
    /// An unwritten edit or a pending conflict keeps the vault in memory, since
    /// dropping it would lose what the user has not saved.
    pub fn park(&mut self) {
        // A file that is not there has nothing to reload: keeping the vault is
        // the only safe answer, and a test fixture is not a box on disk.
        if self.parked || self.dirty || self.conflict || !self.path.is_file() {
            return;
        }
        self.parked_seq = self.vault.seq;
        let name = self.vault.name.clone();
        self.vault = Vault::new(&name);
        self.parked = true;
    }

    /// Bring a parked tab back from its file. The key is the one the session
    /// kept, so no password is asked; ids that no longer exist are dropped.
    pub fn load_parked(&mut self) -> Result<(), String> {
        if !self.parked {
            return Ok(());
        }
        let vault = store::load(&self.path, &self.creds)?;
        self.vault = vault;
        self.parked = false;
        self.parked_seq = self.vault.seq;
        self.sanitize();
        self.dirty = false;
        self.last_edit = None;
        self.conflict = false;
        self.ignore_mtime = None;
        self.last_mtime = store::modified_time(&self.path);
        Ok(())
    }

    /// Drop the ids that no longer exist — they can only go stale after a reload
    /// that replaced the whole vault with another device's copy.
    pub fn sanitize(&mut self) {
        let vault = &self.vault;
        let folder = |f: &String| vault.folders.iter().any(|x| &x.id == f);
        drop_stale(&mut self.folder, folder);
        drop_stale(&mut self.selected, |id| {
            vault.has(id) || vault.folders.iter().any(|x| &x.id == id)
        });
        drop_stale(&mut self.editing_folder, folder);
        drop_stale(&mut self.editing_entry, |id| vault.has(id));
        drop_stale(&mut self.moving_entry, |id| vault.has(id));
        if self.confirm.as_ref().is_some_and(|c| match c {
            Confirm::Entry(id) => !vault.has(id),
            Confirm::Folder(id) => !vault.folders.iter().any(|f| &f.id == id),
            Confirm::Export { .. }
            | Confirm::File { .. }
            | Confirm::Box
            | Confirm::Reload
            | Confirm::EmptyTrash
            | Confirm::RestoreVersion => false,
        }) {
            self.confirm = None;
        }
        drop_stale(&mut self.last_delete, |id| {
            vault.trash.iter().any(|t| t.record.id() == id)
        });
    }
}

/// Clear a slot whose id no longer names anything.
fn drop_stale<T>(slot: &mut Option<T>, exists: impl Fn(&T) -> bool) {
    if slot.as_ref().is_some_and(|value| !exists(value)) {
        *slot = None;
    }
}
