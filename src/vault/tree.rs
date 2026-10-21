//! Walks of the folder tree. Every walk is cycle-safe: a hand-edited or merged box
//! can hold a folder loop, and a listing must terminate.

use crate::vault::model::{Entry, Folder, Vault};
use std::collections::HashSet;

/// The vault's own order — the order the user dragged, not alphabetical.
pub fn children<'a>(vault: &'a Vault, parent: Option<&str>) -> Vec<&'a Folder> {
    vault
        .folders
        .iter()
        .filter(|f| f.parent.as_deref() == parent)
        .collect()
}

/// A folder's drive entries in drag order; a login never lives in one.
pub fn entries<'a>(vault: &'a Vault, parent: Option<&str>) -> Vec<&'a Entry> {
    vault
        .drive
        .iter()
        .filter(|i| i.folder() == parent)
        .collect()
}

pub fn path<'a>(vault: &'a Vault, folder: Option<&str>) -> Vec<&'a Folder> {
    let mut out: Vec<&Folder> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = folder;
    while let Some(id) = cur {
        if !seen.insert(id) {
            break;
        }
        let Some(f) = vault.folders.iter().find(|f| f.id == id) else {
            break;
        };
        out.push(f);
        cur = f.parent.as_deref();
    }
    out.reverse();
    out
}

pub fn tree(vault: &Vault) -> Vec<(usize, &Folder)> {
    const CAP: usize = 32;

    fn walk<'a>(
        vault: &'a Vault,
        parent: Option<&str>,
        depth: usize,
        out: &mut Vec<(usize, &'a Folder)>,
    ) {
        if depth > CAP {
            return;
        }
        for f in children(vault, parent) {
            out.push((depth, f));
            walk(vault, Some(&f.id), depth + 1, out);
        }
    }

    let mut out = Vec::new();
    walk(vault, None, 0, &mut out);
    out
}

pub fn visible<'a>(
    vault: &'a Vault,
    collapsed: &HashSet<String>,
) -> Vec<(usize, &'a Folder, bool)> {
    const CAP: usize = 32;

    fn walk<'a>(
        vault: &'a Vault,
        parent: Option<&str>,
        depth: usize,
        collapsed: &HashSet<String>,
        seen: &mut HashSet<String>,
        out: &mut Vec<(usize, &'a Folder, bool)>,
    ) {
        if depth > CAP {
            return;
        }
        for f in children(vault, parent) {
            if !seen.insert(f.id.clone()) {
                continue;
            }
            let kids = !children(vault, Some(&f.id)).is_empty();
            out.push((depth, f, kids));
            if kids && !collapsed.contains(&f.id) {
                walk(vault, Some(&f.id), depth + 1, collapsed, seen, out);
            }
        }
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(vault, None, 0, collapsed, &mut seen, &mut out);
    out
}

/// Moving `root` under `id` would detach that branch from the tree.
pub fn below(vault: &Vault, root: &str, id: Option<&str>) -> bool {
    path(vault, id).iter().any(|f| f.id == root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{Entry, Login, Note};

    fn vault() -> Vault {
        let mut v = Vault::new("t");
        let mut root = Folder::new("Docs", None, 1);
        root.id = "root".to_owned();
        let mut child = Folder::new("Deep", Some(root.id.clone()), 1);
        child.id = "child".to_owned();
        let mut note = Note::new(1);
        note.folder = Some(root.id.clone());
        v.folders = vec![root, child];
        v.drive.push(Entry::Note(note));
        v
    }

    #[test]
    fn path_walks_up_to_the_root() {
        let v = vault();
        let chain: Vec<&str> = path(&v, Some("child"))
            .iter()
            .map(|f| f.id.as_str())
            .collect();
        assert_eq!(chain, ["root", "child"]);
        assert!(path(&v, None).is_empty());
        assert!(path(&v, Some("ghost")).is_empty());
    }

    #[test]
    fn path_survives_a_cycle() {
        let mut v = vault();
        v.folders[0].parent = Some("child".to_owned());
        let chain: Vec<&str> = path(&v, Some("root"))
            .iter()
            .map(|f| f.id.as_str())
            .collect();
        assert_eq!(chain, ["child", "root"]);
    }

    #[test]
    fn below_detects_descendants() {
        let v = vault();
        assert!(below(&v, "root", Some("child")));
        assert!(!below(&v, "child", Some("root")));
        assert!(below(&v, "child", Some("child")));
    }

    #[test]
    fn entries_are_the_notes_and_files_of_the_folder() {
        let mut v = vault();
        v.logins.push(Login::new(1));
        assert_eq!(v.drive.len(), 1);
        assert_eq!(entries(&v, Some("root")).len(), 1);
        assert!(entries(&v, None).is_empty());
    }

    #[test]
    fn visible_hides_the_children_of_a_collapsed_folder() {
        let v = vault();
        let expanded = visible(&v, &HashSet::new());
        let chain: Vec<(&str, usize)> = expanded
            .iter()
            .map(|(d, f, _)| (f.id.as_str(), *d))
            .collect();
        assert_eq!(chain, [("root", 0), ("child", 1)]);
        assert!(expanded[0].2, "root has children");

        let mut folded = HashSet::new();
        folded.insert("root".to_owned());
        let rows = visible(&v, &folded);
        let ids: Vec<&str> = rows.iter().map(|(_, f, _)| f.id.as_str()).collect();
        assert_eq!(ids, ["root"], "a collapsed folder hides its subtree");
    }

    #[test]
    fn tree_nests_and_survives_a_cycle() {
        let v = vault();
        let ids: Vec<(&str, usize)> = tree(&v).iter().map(|(d, f)| (f.id.as_str(), *d)).collect();
        assert_eq!(ids, [("root", 0), ("child", 1)]);

        let mut cyc = vault();
        let mut top = Folder::new("Top", None, 1);
        top.id = "top".to_owned();
        cyc.folders.push(top);
        cyc.folders[0].parent = Some("child".to_owned());
        let walked = tree(&cyc);
        assert_eq!(walked[0].1.id, "top");
        assert!(walked.len() < 40, "cycle must terminate: {}", walked.len());
    }
}
