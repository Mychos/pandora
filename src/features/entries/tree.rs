//! The passwords pane's generated folders: what the folders view draws and what
//! each folder holds. Pure functions over the entries on screen, so a search
//! narrows the tree the same way it narrows the list. Nothing here is persisted:
//! the folders are read off the entries every frame.

use crate::features::entries::model::Axis;
use crate::shared::i18n::t;
use crate::shared::theme;
use crate::vault::matching;
use crate::vault::model::Login;

/// A generated folder: the path that names it, the entries it holds and the mark
/// its entries suggest.
#[derive(Debug, PartialEq)]
pub struct Node {
    /// Unique across axes: the axis's own name first, so two axes never share a
    /// same-named node.
    pub key: String,
    /// The axis segment that leads here, so a breadcrumb can rebuild the path.
    pub path: Vec<String>,
    pub label: String,
    /// The brand of the first entry that carries one: what the row wears. Only a
    /// site folder ever claims one — a username is nobody's brand.
    pub brand: Option<theme::Brand>,
    pub ids: Vec<String>,
}

impl Node {
    fn new(key: String, path: Vec<String>, label: String) -> Self {
        Self {
            key,
            path,
            label,
            brand: None,
            ids: Vec::new(),
        }
    }

    /// How many entries hang under this folder.
    pub fn total(&self) -> usize {
        self.ids.len()
    }
}

/// The key a node at `path` carries: the axis's own name first, so two axes
/// never share a key.
pub fn node_key(axis: Axis, path: &[String]) -> String {
    format!("{}/{}", axis.key(), path.join("/"))
}

/// The site an entry claims: its registrable domain, or its host when it has no
/// eTLD+1. `no site` when none of its addresses parse.
fn site(item: &Login) -> String {
    item.uris()
        .find_map(|(url, _)| matching::parse(url))
        .map(|site| site.registrable.unwrap_or(site.host))
        .unwrap_or_else(|| t("no site").to_owned())
}

fn user(item: &Login) -> String {
    let user = item.username.trim().to_lowercase();
    if user.is_empty() {
        t("no user").to_owned()
    } else {
        user
    }
}

/// The folder label an entry sits under, for the chosen axis.
fn label(item: &Login, axis: Axis) -> String {
    match axis {
        Axis::Target => site(item),
        Axis::User => user(item),
    }
}

/// The generated tree: one folder per axis value, alphabetically. Nothing is
/// written to the vault, and a search only hands in the entries it kept.
pub fn folders(items: &[&Login], axis: Axis) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in items {
        let label = label(item, axis);
        let index = *at.entry(label.clone()).or_insert_with(|| {
            let path = vec![label.clone()];
            roots.push(Node::new(node_key(axis, &path), path, label.clone()));
            roots.len() - 1
        });
        let node = &mut roots[index];
        if axis == Axis::Target && node.brand.is_none() && label != t("no site") {
            node.brand = theme::brand_of(item);
        }
        node.ids.push(item.id.clone());
    }
    roots.sort_by_cached_key(|node| (rank(&node.label), node.label.clone()));
    roots
}

/// Where a label sorts: a real one first, then the two the tree invents for a
/// record that named nothing, `no user` before `no site`.
fn rank(label: &str) -> u8 {
    if label == t("no user") {
        1
    } else if label == t("no site") {
        2
    } else {
        0
    }
}

/// The folder carrying this key.
pub fn find<'a>(nodes: &'a [Node], key: &str) -> Option<&'a Node> {
    nodes.iter().find(|node| node.key == key)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, user: &str, url: &str) -> Login {
        let mut it = Login::new(0);
        it.name = name.to_owned();
        it.username = user.to_owned();
        it.url = url.to_owned();
        it
    }

    #[test]
    fn a_site_gathers_its_subdomains_and_ignores_user_case() {
        let a = item("GitHub", "octocat", "https://github.com/login");
        let b = item("Gist", "OctoCat ", "gist.github.com");
        let roots = folders(&[&a, &b], Axis::Target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].label, "github.com");
        assert_eq!(roots[0].total(), 2);
        assert_eq!(roots[0].ids, vec![a.id.clone(), b.id.clone()]);
    }

    #[test]
    fn a_second_user_grows_a_sibling_folder() {
        let a = item("Work", "work", "github.com");
        let b = item("Personal", "me", "github.com");
        let roots = folders(&[&a, &b], Axis::User);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].label, "me");
        assert_eq!(roots[1].label, "work");
        assert_eq!(roots[0].ids, vec![b.id.clone()]);
    }

    #[test]
    fn the_axis_picks_the_level() {
        let a = item("A", "u", "example.com");
        assert_eq!(folders(&[&a], Axis::Target)[0].label, "example.com");
        assert_eq!(folders(&[&a], Axis::User)[0].label, "u");
    }

    #[test]
    fn a_site_folder_wears_the_brand_its_entries_suggest() {
        let a = item("GitHub", "octocat", "https://github.com/login");
        let b = item("Gist", "x", "gist.github.com");
        let roots = folders(&[&b, &a], Axis::Target);
        assert_eq!(roots[0].label, "github.com");
        assert_eq!(roots[0].brand, Some(theme::Brand::Github));

        let mut pinned = item("Shop", "me", "https://example.com");
        pinned.icon = Some("stripe".to_owned());
        assert_eq!(
            folders(&[&pinned], Axis::Target)[0].brand,
            Some(theme::Brand::Stripe),
            "an entry's own icon wins over its url"
        );

        assert_eq!(
            folders(&[&a], Axis::User)[0].brand,
            None,
            "a username is nobody's brand"
        );
    }

    #[test]
    fn entries_without_a_site_or_a_user_still_gather() {
        let a = item("Bare", "", "");
        let roots = folders(&[&a], Axis::Target);
        assert_eq!(roots[0].label, "no site");
        assert_eq!(roots[0].ids, vec![a.id.clone()]);
        assert_eq!(folders(&[&a], Axis::User)[0].label, "no user");
    }

    #[test]
    fn find_reaches_a_folder_by_its_key() {
        let a = item("Work", "work", "github.com");
        let roots = folders(&[&a], Axis::Target);
        assert_eq!(find(&roots, &roots[0].key).unwrap(), &roots[0]);
        assert!(
            find(&roots, "user/work").is_none(),
            "another axis is elsewhere"
        );
    }
}
