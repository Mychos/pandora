//! The CSV interchange file: one writer and one reader over the same header, so
//! what this app exports is exactly what it can read back. It is plaintext, and
//! it also owns where an export lands (`download_dir`, `safe_name`).
//!
//! Both platforms import. A desktop's picker hands back the user's own file; a
//! phone's picker leaves a copy in the app's own storage, which `import_copy`
//! reads and removes.

use crate::app::{App, StatusKind};
use crate::vault::model::{now_secs, Any, Entry, Folder, Kind, Login, Note, Otp, Vault};
use crate::vault::store;
use crate::vault::tree as folders;
use std::mem;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Every name here is also an alias the reader accepts, so an exported file
/// always imports back. `kind` is what tells the four collections apart: the
/// vault itself writes no tag.
pub const HEADER: &str = "name,username,password,url,notes,folder,kind,totp";

/// Anything bigger is not a CSV export, and this file comes from outside.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Deepest folder path an import will create — the same ceiling as the drive tree.
const DEPTH: usize = 32;

/// Downloads when the platform has one, else the data dir the boxes live in — an
/// export is never silently a no-op.
pub fn download_dir() -> Result<PathBuf, String> {
    dirs::download_dir()
        .or_else(dirs::data_local_dir)
        .ok_or_else(|| crate::shared::i18n::t("no downloads folder found").to_string())
}

pub fn write(vault: &Vault) -> Zeroizing<String> {
    let mut out = Zeroizing::new(format!("{HEADER}\n"));
    for it in vault.records() {
        let folder = folder_path(vault, it.folder());
        // Only the columns a record actually carries are filled: a note has a
        // body and a folder, a login has its credentials, a code has its seed. A
        // file carries bytes a text file cannot hold, so it is not a row at all.
        let (username, password, url, notes, totp) = match it {
            Any::Login(it) => (
                it.username.as_str(),
                it.password.as_str(),
                it.url.as_str(),
                it.notes.as_str(),
                "",
            ),
            Any::Drive(Entry::Note(it)) => ("", "", "", it.body.as_str(), ""),
            Any::Drive(Entry::File(_)) => continue,
            Any::Otp(it) => ("", "", "", "", it.secret.as_str()),
        };
        for value in [
            it.name(),
            username,
            password,
            url,
            notes,
            folder.as_str(),
            it.kind().label(),
            totp,
        ] {
            cell(&mut out, value);
        }
        out.pop(); // the row ends with a newline, not a trailing comma
        out.push('\n');
    }
    out
}

fn cell(out: &mut String, value: &str) {
    if value.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(value);
    }
    out.push(',');
}

/// RFC 4180 rows with a UTF-8 BOM ignored. A stray quote in an unquoted cell is
/// literal; an unterminated quote runs to the end of the file instead of panicking.
pub fn parse(text: &str) -> Vec<Vec<String>> {
    parse_delimited(text, ',')
}

fn parse_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    // Has this row seen anything? A blank line is dropped, a row of empty cells is not.
    let mut started = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => {
                quoted = true;
                started = true;
            }
            c if c == delimiter => {
                cells.push(mem::take(&mut cell));
                started = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if started || !cell.is_empty() || !cells.is_empty() {
                    cells.push(mem::take(&mut cell));
                    rows.push(mem::take(&mut cells));
                }
                started = false;
            }
            _ => {
                cell.push(c);
                started = true;
            }
        }
    }
    if started || !cell.is_empty() || !cells.is_empty() {
        cells.push(cell);
        rows.push(cells);
    }
    rows
}

/// Every column is optional: only `name` or `password` is required for the file
/// to be recognised at all, and a field it does not carry lands empty.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Columns {
    name: Option<usize>,
    username: Option<usize>,
    password: Option<usize>,
    url: Option<usize>,
    notes: Option<usize>,
    folder: Option<usize>,
    kind: Option<usize>,
    totp: Option<usize>,
}

impl Columns {
    /// Match a header row against the aliases this app, KeePassXC, KeePass and
    /// Bitwarden write. A header is compared with its spaces and underscores
    /// gone, so `Login Name` and `login_name` are the same column. Columns
    /// nobody reads are not looked at, so their export imports as it stands.
    fn detect(header: &[String]) -> Result<Self, String> {
        let find = |aliases: &[&str]| {
            header.iter().position(|h| {
                let h: String = h
                    .trim()
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .flat_map(char::to_lowercase)
                    .collect();
                aliases.contains(&h.as_str())
            })
        };
        let cols = Self {
            name: find(&["name", "title", "account"]),
            username: find(&["username", "user", "login", "loginusername", "loginname"]),
            password: find(&["password", "loginpassword"]),
            url: find(&["url", "uri", "loginuri", "website"]),
            notes: find(&["notes", "note", "comments"]),
            folder: find(&["folder", "group", "grouping", "path"]),
            kind: find(&["type", "kind"]),
            totp: find(&["totp", "totpsecret", "otp"]),
        };
        if cols.name.is_none() && cols.password.is_none() {
            return Err(crate::shared::i18n::tf(
                "no recognizable columns — this app reads {header}",
                &[("header", &HEADER)],
            ));
        }
        Ok(cols)
    }

    fn score(self) -> usize {
        [
            self.name,
            self.username,
            self.password,
            self.url,
            self.notes,
            self.folder,
            self.kind,
            self.totp,
        ]
        .into_iter()
        .flatten()
        .count()
    }

    fn value(row: &[String], at: Option<usize>) -> &str {
        at.and_then(|i| row.get(i)).map_or("", String::as_str)
    }

    fn cell<'r>(&self, row: &'r [String], at: Option<usize>) -> &'r str {
        Self::value(row, at).trim()
    }

    /// What the row is: the tag says it, and a row that says nothing is a login,
    /// which is what every manager's export without a type means.
    fn kind(&self, row: &[String]) -> Kind {
        match self.cell(row, self.kind).to_ascii_lowercase().as_str() {
            "note" | "securenote" | "secure note" | "document" | "file" => Kind::Note,
            "otp" | "totp" => Kind::Otp,
            _ => Kind::Login,
        }
    }

    fn name(&self, row: &[String], fallback: &str) -> String {
        match self.cell(row, self.name) {
            "" => fallback.to_owned(),
            name => name.to_owned(),
        }
    }

    fn login(&self, row: &[String]) -> Login {
        let mut it = Login::new(now_secs());
        it.name = self.name(row, &it.name);
        it.username = self.cell(row, self.username).to_owned();
        it.password = Zeroizing::new(self.cell(row, self.password).to_owned());
        it.url = self.cell(row, self.url).to_owned();
        it.notes = Zeroizing::new(self.cell(row, self.notes).to_owned());
        it
    }

    fn note(&self, row: &[String]) -> Note {
        let mut it = Note::new(now_secs());
        it.name = self.name(row, &it.name);
        it.body = Zeroizing::new(self.cell(row, self.notes).to_owned());
        it
    }

    fn otp(&self, row: &[String]) -> Otp {
        let mut it = Otp::new(now_secs());
        it.name = self.name(row, &it.name);
        it.secret = Zeroizing::new(self.cell(row, self.totp).to_owned());
        it
    }
}

/// The `a/b` path a record's folder names, in the terms the file writes it, so a
/// re-import can tell whether a row is already there.
fn folder_path(vault: &Vault, folder: Option<&str>) -> String {
    folders::path(vault, folder)
        .iter()
        .map(|f| f.name.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// The folder an `a/b/c` path names, created level by level; None for an empty
/// path — the drive root. Only a note lives in a folder.
fn find_or_create_folder(vault: &mut Vault, path: &str) -> Option<String> {
    let mut parent: Option<String> = None;
    for name in path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(DEPTH)
    {
        let found = folders::children(vault, parent.as_deref())
            .into_iter()
            .find(|f| f.name == name)
            .map(|f| f.id.clone());
        parent = Some(match found {
            Some(id) => id,
            None => {
                let folder = Folder::new(name, parent.clone(), now_secs());
                let id = folder.id.clone();
                vault.folders.push(folder);
                id
            }
        });
    }
    parent
}

/// How an export is named: the box's own name, or one that says nothing at all.
/// The prompt asks which, because a file called after the box tells whoever reads
/// the folder what is in it — and the file itself is already in the clear.
pub fn export_name(box_name: &str, neutral: bool) -> String {
    match neutral {
        true => "pandora-export.csv".to_owned(),
        false => {
            let stem = box_name.trim();
            let stem = if stem.is_empty() { "pandora" } else { stem };
            format!("{}.csv", store::sanitize(stem))
        }
    }
}

impl App {
    /// Write the whole box out as a plaintext CSV (*not* re-encrypted — careful!).
    ///
    /// `neutral` is the name the file goes out under, and the prompt is where that
    /// is chosen: an export is the one way out of a box, and a file called after
    /// the box says on its own what is inside it.
    pub fn export_csv(&mut self, neutral: bool) {
        let (name, text) = match self.boxes.session.as_ref() {
            Some(s) => (s.vault.name.clone(), write(&s.vault)),
            None => return,
        };
        let ok = (|| -> Result<PathBuf, String> {
            let out = download_dir()?.join(export_name(&name, neutral));
            store::write_private(&out, text.as_bytes())?;
            Ok(out)
        })();
        match ok {
            Ok(p) => self.shell.set_status(
                crate::shared::i18n::tf(
                    "exported to {path} — plaintext, keep it safe",
                    &[("path", &p.display())],
                ),
                StatusKind::Saved,
            ),
            Err(e) => self.shell.fail(crate::shared::i18n::tf(
                "export failed: {error}",
                &[("error", &e)],
            )),
        }
    }

    /// Read a CSV the user picks into the box on screen. Additive, never
    /// destructive: nothing already in the box is overwritten.
    ///
    /// The phone's picker belongs to the activity and blocks the frame while it
    /// is up, exactly as the drive's does. `*/*` because a provider can announce
    /// a CSV as `application/vnd.ms-excel` or `application/octet-stream`; what
    /// validates it is the size cap and the header, not the MIME.
    #[cfg(target_os = "android")]
    pub fn import_csv(&mut self) {
        match crate::shared::android::text_with("pick", "*/*") {
            Ok(Some(path)) => self.import_copy(Path::new(&path)),
            Ok(None) => {}
            Err(e) => self.shell.fail(e),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn import_csv(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv", "txt"])
            .pick_file()
        else {
            return;
        };
        // Not `import_copy`: a desktop picker hands back the user's own file,
        // never a copy of it, and the file is theirs to keep.
        let read = self.read_csv(&path);
        self.report(read);
    }

    /// Read the copy a phone's picker made, then drop it whatever happened: the
    /// file the user picked was never touched, and a plaintext CSV left in the
    /// clear would outlive the box. Not `cfg`'d, so the round trip is testable
    /// off the phone.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn import_copy(&mut self, path: &Path) {
        let read = self.read_csv(path);
        let _ = std::fs::remove_file(path);
        self.report(read);
    }

    /// What an import says on the status line, on either platform.
    fn report(&mut self, read: Result<(usize, usize), String>) {
        match read {
            Ok((0, skipped)) => self.shell.set_status(
                format!("nothing to import — {skipped} entries are already in the box"),
                StatusKind::Info,
            ),
            Ok((added, skipped)) => self.shell.set_status(
                crate::shared::i18n::tf(
                    "imported {added} entries ({skipped} already there)",
                    &[("added", &added), ("skipped", &skipped)],
                ),
                StatusKind::Saved,
            ),
            Err(e) => self.shell.fail(crate::shared::i18n::tf(
                "import failed: {error}",
                &[("error", &e)],
            )),
        }
    }

    /// The size is checked before the text, so an oversized file is never pulled
    /// into memory.
    fn read_csv(&mut self, path: &Path) -> Result<(usize, usize), String> {
        let len = std::fs::metadata(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .len();
        if len > MAX_BYTES {
            return Err("that file is too big to be a CSV export".to_owned());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        self.absorb(&text)
    }

    /// Every row into the box on screen, header first. Reports how many records
    /// were added and how many rows only repeated one already there.
    fn absorb(&mut self, text: &str) -> Result<(usize, usize), String> {
        let mut best: Option<(usize, usize, Vec<Vec<String>>, Columns)> = None;
        for (priority, delimiter) in [',', ';', '\t'].into_iter().enumerate() {
            let rows = parse_delimited(text, delimiter);
            let Some(header) = rows.first() else { continue };
            let Ok(cols) = Columns::detect(header) else {
                continue;
            };
            let score = cols.score();
            if best
                .as_ref()
                .is_none_or(|(best_score, best_priority, _, _)| {
                    score > *best_score || (score == *best_score && priority < *best_priority)
                })
            {
                best = Some((score, priority, rows, cols));
            }
        }
        let Some((_, _, mut rows, cols)) = best else {
            if parse(text).is_empty() {
                return Err(crate::shared::i18n::t("that file has no rows").to_owned());
            }
            return Err(crate::shared::i18n::tf(
                "no recognizable columns — this app reads {header}",
                &[("header", &HEADER)],
            ));
        };
        if rows.is_empty() {
            return Err(crate::shared::i18n::t("that file has no rows").to_owned());
        }
        rows.remove(0);
        let Some(s) = self.boxes.session.as_mut() else {
            return Err("no box is open".to_owned());
        };

        let (mut added, mut skipped) = (0, 0);
        for row in &rows {
            if row.iter().all(|c| c.trim().is_empty()) {
                continue; // a blank line inside the file, not an entry
            }
            {
                let mut land = |fresh: bool| {
                    if fresh {
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                };
                match cols.kind(row) {
                    Kind::Otp => {
                        let it = cols.otp(row);
                        if it.secret.is_empty()
                            || s.vault
                                .otps
                                .iter()
                                .any(|e| e.name == it.name && e.secret == it.secret)
                        {
                            land(false);
                            continue;
                        }
                        land(true);
                        s.vault.otps.push(it);
                    }
                    Kind::Note | Kind::File => {
                        let mut it = cols.note(row);
                        let path = cols.cell(row, cols.folder).to_owned();
                        let found = s.vault.drive.iter().any(|e| {
                            e.name() == it.name
                                && matches!(e, Entry::Note(n) if n.body == it.body)
                                && folder_path(&s.vault, e.folder()) == path
                        });
                        if found {
                            land(false);
                            continue;
                        }
                        it.folder = find_or_create_folder(&mut s.vault, &path);
                        land(true);
                        s.vault.drive.push(Entry::Note(it));
                    }
                    Kind::Login => {
                        let it = cols.login(row);
                        if s.vault.logins.iter().any(|e| {
                            e.name == it.name
                                && e.username == it.username
                                && e.password == it.password
                        }) {
                            land(false);
                            continue;
                        }
                        land(true);
                        s.vault.logins.push(it);
                    }
                }
            }
            // A manager that carries its one-time seed on the login's own row
            // (KeePassXC does) leaves it here as well, so it is not lost.
            let totp = cols.cell(row, cols.totp);
            if cols.kind(row) == Kind::Login && !totp.is_empty() {
                let it = cols.otp(row);
                let fresh = !s
                    .vault
                    .otps
                    .iter()
                    .any(|e| e.name == it.name && e.secret == it.secret);
                if fresh {
                    s.vault.otps.push(it);
                    added += 1;
                }
            }
        }
        if added > 0 {
            s.mark_dirty();
        }
        Ok((added, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An export is named after the box unless the user says otherwise: a file
    /// called `Divorcio.csv` in Downloads says on its own what it holds, and it is
    /// already unencrypted there.
    #[test]
    fn an_export_is_named_after_the_box_or_after_nothing() {
        assert_eq!(export_name("Work", false), "Work.csv");
        assert_eq!(export_name("  Work  ", false), "Work.csv");
        assert_eq!(export_name("", false), "pandora.csv", "no name at all");
        assert_eq!(
            export_name("a/b", false),
            "a_b.csv",
            "and never a path through the name"
        );

        let blank = export_name("Divorcio", true);
        assert_eq!(blank, "pandora-export.csv");
        assert!(!blank.contains("Divorcio"));
    }

    const KEEPASSXC: &str = "Group,Title,Username,Password,URL,Notes\nWork,GitHub,octocat,hunter2,https://github.com,ssh key";
    const BITWARDEN: &str = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\nPersonal,0,login,Mail,,,0,https://mail.example,me@example,pw,\nPersonal,0,note,Ideas,\"one, two\",,0,,,,";

    fn file(text: &str) -> Vec<Vec<String>> {
        parse(text)
    }

    fn app() -> App {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("pandora");
        app
    }

    #[test]
    fn quoted_cells_survive_commas_quotes_and_newlines() {
        assert_eq!(
            file("a,\"b,c\",\"say \"\"hi\"\"\"\n"),
            vec![vec!["a", "b,c", "say \"hi\""]]
        );
        assert_eq!(
            file("\"first\nsecond\",x\n"),
            vec![vec!["first\nsecond", "x"]]
        );
    }

    #[test]
    fn crlf_a_bom_blank_lines_and_short_rows_are_handled() {
        assert_eq!(
            file("\u{feff}a,b\r\n\r\nc\r\n"),
            vec![vec!["a", "b"], vec!["c"]]
        );
        assert_eq!(file("a,,,\n"), vec![vec!["a", "", "", ""]]);
        assert_eq!(file(""), Vec::<Vec<String>>::new());
        assert_eq!(file("\"open,never closed"), vec![vec!["open,never closed"]]);
    }

    #[test]
    fn the_three_headers_are_recognised_and_a_stranger_is_refused() {
        let ours = Columns::detect(&file(HEADER)[0]).unwrap();
        assert_eq!(ours.kind, Some(6));
        assert_eq!(ours.totp, Some(7));

        let keepass = Columns::detect(&file(KEEPASSXC)[0]).unwrap();
        assert_eq!(
            (keepass.name, keepass.username, keepass.password),
            (Some(1), Some(2), Some(3))
        );
        assert_eq!(keepass.folder, Some(0), "Group is the folder");
        assert_eq!(keepass.kind, None, "KeePassXC does not say what a row is");

        let bitwarden = Columns::detect(&file(BITWARDEN)[0]).unwrap();
        assert_eq!(bitwarden.folder, Some(0));
        assert_eq!((bitwarden.name, bitwarden.kind), (Some(3), Some(2)));
        assert!(bitwarden.url.is_some() && bitwarden.username.is_some());

        assert!(
            Columns::detect(&file("a,b,c")[0]).is_err(),
            "no name, no password"
        );
    }

    #[test]
    fn a_keepass_file_becomes_a_login_with_its_group_named() {
        let rows = file(KEEPASSXC);
        let cols = Columns::detect(&rows[0]).unwrap();
        let it = cols.login(&rows[1]);
        assert_eq!(it.name, "GitHub");
        assert_eq!(it.username, "octocat");
        assert_eq!(it.password.as_str(), "hunter2");
        assert_eq!(it.url, "https://github.com");
        assert_eq!(it.notes.as_str(), "ssh key");
        assert_eq!(cols.kind(&rows[1]), Kind::Login, "no type column: a login");
    }

    #[test]
    fn keepass_headers_with_spaces_and_semicolons_are_detected() {
        let mut app = app();
        assert_eq!(
            app.absorb(
                "Account;Login Name;Password;Web Site;Comments\nMail;me@example.com;pw;https://mail.example;note"
            )
            .unwrap(),
            (1, 0)
        );
        let it = &app.boxes.session.as_ref().unwrap().vault.logins[0];
        assert_eq!(it.name, "Mail");
        assert_eq!(it.username, "me@example.com");
        assert_eq!(it.password.as_str(), "pw");
        assert_eq!(it.url, "https://mail.example");
        assert_eq!(it.notes.as_str(), "note");
    }

    /// The type column is what sends a row to its collection, and a manager that
    /// writes no type at all means a login.
    #[test]
    fn the_type_column_picks_the_collection() {
        let mut app = app();
        assert_eq!(
            app.absorb(
                "name,password,kind,notes,totp\n\
                 GitHub,octocat,password,,\n\
                 Ideas,,note,\"one, two\",\n\
                 Codes,,otp,,JBSWY3DPEHPK3PXP\n\
                 Recibo,,securenote,pagado,"
            )
            .unwrap(),
            (4, 0)
        );
        let v = &app.boxes.session.as_ref().unwrap().vault;
        assert_eq!(v.logins.len(), 1);
        assert_eq!(v.drive.len(), 2, "a note and a secure note are both notes");
        assert_eq!(v.otps.len(), 1);
        assert_eq!(v.otps[0].name, "Codes");
        assert_eq!(v.drive[0].name(), "Ideas");
        assert!(
            matches!(&v.drive[0], Entry::Note(n) if n.body.as_str() == "one, two"),
            "the body came back on the note"
        );
    }

    #[test]
    fn a_folder_path_becomes_a_chain_and_is_reused() {
        let mut vault = Vault::new("Box");
        let deep = find_or_create_folder(&mut vault, "Personal\\Work/2024").unwrap();
        assert_eq!(vault.folders.len(), 3);
        let again = find_or_create_folder(&mut vault, "/Personal//Work/").unwrap();
        assert_eq!(vault.folders.len(), 3, "the same path is the same folders");
        assert_eq!(again, vault.folders[1].id, "and the last level found");
        assert_ne!(deep, again);

        assert_eq!(
            find_or_create_folder(&mut vault, "   "),
            None,
            "no path, no folder"
        );
        assert_eq!(vault.folders.len(), 3);
    }

    /// What the writer produces, the reader takes back — folders and all.
    #[test]
    fn a_written_file_imports_back_as_the_same_box() {
        let mut app = app();
        let text = "name,username,password,url,notes,folder,kind\n\
                    GitHub,octocat,hunter2,https://github.com,ssh key,,password\n\
                    \"Ideas, round one\",,,,\"first\nsecond\",Personal/Work,note";
        assert_eq!(app.absorb(text).unwrap(), (2, 0));

        let out = write(&app.boxes.session.as_ref().unwrap().vault);
        assert_eq!(
            app.absorb(&out).unwrap(),
            (0, 2),
            "a round trip adds nothing"
        );

        let vault = &app.boxes.session.as_ref().unwrap().vault;
        assert_eq!(vault.logins.len(), 1);
        assert_eq!(vault.drive.len(), 1);
        let note = match &vault.drive[0] {
            Entry::Note(n) => n,
            Entry::File(_) => panic!("a row of text is a note"),
        };
        assert_eq!(
            note.body.as_str(),
            "first\nsecond",
            "the line break came back"
        );
        let folder = vault
            .folders
            .iter()
            .find(|f| f.id == note.folder.clone().unwrap())
            .unwrap();
        assert_eq!(folder.name, "Work");
        assert_eq!(
            folders::path(vault, note.folder.as_deref())
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Personal", "Work"]
        );
    }

    /// Importing twice changes nothing: the box keeps one copy of each record.
    #[test]
    fn an_entry_already_there_is_skipped_not_duplicated() {
        let mut app = app();
        assert_eq!(app.absorb(KEEPASSXC).unwrap(), (1, 0));
        assert_eq!(app.absorb(KEEPASSXC).unwrap(), (0, 1));
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.logins.len(), 1);

        // the same name with a new password is a new entry, not a duplicate
        assert_eq!(
            app.absorb(&KEEPASSXC.replace("hunter2", "hunter3"))
                .unwrap(),
            (1, 0)
        );
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.logins.len(), 2);
    }

    /// KeePassXC carries the seed on the login's own row: the login and its code
    /// both land, as two records, and a re-import adds neither again.
    #[test]
    fn a_keepassxc_row_brings_its_seed_as_a_code() {
        let mut app = app();
        let source = "Group,Title,Username,Password,URL,Notes,TOTP\n\
                      Work,GitHub,octocat,hunter2,https://github.com,ssh key,JBSWY3DPEHPK3PXP";
        assert_eq!(app.absorb(source).unwrap(), (2, 0), "a login and its code");

        let v = &app.boxes.session.as_ref().unwrap().vault;
        assert_eq!(v.logins.len(), 1);
        assert_eq!(v.otps.len(), 1);
        assert_eq!(v.otps[0].name, "GitHub");
        assert_eq!(v.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");

        assert_eq!(app.absorb(source).unwrap(), (0, 1), "nothing lands twice");
        {
            let v = &app.boxes.session.as_ref().unwrap().vault;
            assert_eq!((v.logins.len(), v.otps.len()), (1, 1));
            let out = write(v);
            assert!(out.contains("JBSWY3DPEHPK3PXP"), "{}", out.as_str());
        }
        let out = write(&app.boxes.session.as_ref().unwrap().vault);
        assert_eq!(app.absorb(&out).unwrap(), (0, 2), "and it round-trips");
    }

    #[test]
    fn a_file_that_is_not_a_csv_imports_nothing() {
        let mut app = app();
        assert!(app.absorb("").is_err(), "no rows at all");
        assert!(
            app.absorb("a,b,c\n1,2,3\n").is_err(),
            "no recognizable columns"
        );
        assert_eq!(
            app.absorb("name,password\n,,,\n").unwrap(),
            (0, 0),
            "a row of empty cells is not an entry"
        );
        let v = &app.boxes.session.as_ref().unwrap().vault;
        assert_eq!(v.records().count(), 0);
    }

    #[test]
    fn an_import_larger_than_the_cap_is_refused() {
        let path = std::env::temp_dir().join("pandora-import-too-big.csv");
        std::fs::write(&path, vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        let mut app = app();
        assert!(app.read_csv(&path).unwrap_err().contains("too big"));
        let _ = std::fs::remove_file(&path);
    }

    /// A phone's picker leaves a copy behind, and the copy is ours to remove:
    /// the plaintext file must not outlive the import.
    #[test]
    fn an_imported_copy_is_deleted_after_reading() {
        let path = std::env::temp_dir().join("pandora-import-copy.csv");
        std::fs::write(
            &path,
            b"name,username,password,url,notes,folder,kind\n\
              GitHub,octocat,hunter2,https://github.com,ssh key,,password\n",
        )
        .unwrap();
        let mut app = app();

        app.import_copy(&path);
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.logins.len(), 1);
        assert!(!path.exists(), "the copy is gone once it is read");
    }

    /// The copy goes even when the file is refused, and the refusal is what the
    /// status line says.
    #[test]
    fn a_copy_that_is_too_big_is_removed_too() {
        let path = std::env::temp_dir().join("pandora-import-copy-big.csv");
        std::fs::write(&path, vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        let mut app = app();

        app.import_copy(&path);
        assert!(!path.exists(), "a refused copy is still ours to remove");
        let said = app.shell.status.as_ref().unwrap();
        assert!(
            said.text.contains("too big") && said.kind == StatusKind::Error,
            "{said_text:?}",
            said_text = said.text
        );
    }

    #[test]
    fn an_export_name_keeps_no_path_separator() {
        assert_eq!(store::sanitize("My Box/2024:v1"), "My_Box_2024_v1");
        assert_eq!(store::sanitize("work.pandora"), "work.pandora");
    }

    /// Read by other managers: a comma in a name must not shift every column after it.
    #[test]
    fn an_entry_that_leaves_the_drive_keeps_its_folder_path() {
        let mut vault = Vault::new("Box");
        let parent = Folder::new("Personal", None, 1);
        let child = Folder::new("Work", Some(parent.id.clone()), 1);
        vault.folders = vec![parent.clone(), child.clone()];

        let mut note = Note::new(1);
        note.name = "line1, \"line2\"".to_owned();
        note.body = Zeroizing::new("first\nsecond".to_owned());
        note.folder = Some(child.id.clone());
        vault.drive.push(crate::vault::model::Entry::Note(note));

        let out = write(&vault);
        let out = out.as_str();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(lines[1], "\"line1, \"\"line2\"\"\",,,,\"first");
        assert!(
            out.contains("\"first\nsecond\",Personal/Work,note,\n"),
            "{out}"
        );
        assert_eq!(lines.len(), 3, "the note's line break stays inside a cell");
        assert_eq!(parse(out)[1].len(), 8, "the empty TOTP column is retained");
    }
}
