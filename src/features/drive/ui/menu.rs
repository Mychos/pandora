//! "Move to…" destination list, shared so every entry point offers the same tree.

use crate::shared::theme;
use crate::vault::model::Vault;
use crate::vault::tree as folders;
use egui::{RichText, Ui};

/// Folder id/name/depth, owned so a caller can hold it while the vault is mutably borrowed.
pub type Tree = Vec<(usize, String, String)>;

pub fn tree(vault: &Vault) -> Tree {
    folders::tree(vault)
        .iter()
        .map(|(d, f)| (*d, f.id.clone(), f.name.clone()))
        .collect()
}

/// "top level" (as an icon) plus one row per folder. Returns the picked destination:
/// `Some(None)` is the drive root, `Some(Some(id))` a folder, `None` no pick.
pub fn rows(
    ui: &mut Ui,
    icons: &theme::Icons,
    tree: &Tree,
    current: Option<String>,
) -> Option<Option<String>> {
    let mut dest = None;
    let color = if current.is_none() {
        theme::ACCENT
    } else {
        theme::palette().text
    };
    if current.is_some() {
        let img = icons
            .image(theme::Icon::Up, 15.0, color)
            .map(|i| i.alt_text(crate::shared::i18n::text("top level").into_owned()));
        let mut resp = match img {
            Some(img) => ui.add(
                egui::Button::new(img)
                    .small()
                    .min_size(egui::vec2(0.0, 28.0)),
            ),
            None => ui.add(
                egui::Button::new("")
                    .small()
                    .min_size(egui::vec2(0.0, 28.0)),
            ),
        };
        resp = resp.on_hover_text(crate::shared::i18n::text("top level").into_owned());
        if resp.clicked() {
            dest = Some(None);
            ui.close();
        }
    }
    for (depth, id, name) in tree {
        // The folder the entry already lives in is not a destination.
        if current.as_deref() == Some(id.as_str()) {
            continue;
        }
        let selected = false;
        let label = format!("{}{name}", "  ".repeat(*depth));
        row(ui, &label, Some(id.clone()), selected, &mut dest);
    }
    dest
}

fn row(
    ui: &mut Ui,
    name: &str,
    id: Option<String>,
    selected: bool,
    dest: &mut Option<Option<String>>,
) {
    let color = if selected {
        theme::ACCENT
    } else {
        theme::palette().text
    };
    if ui
        .button(RichText::new(name).small().color(color))
        .clicked()
    {
        *dest = Some(id);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::theme;
    use crate::shared::ui::shot;
    use crate::vault::model::{Folder, Vault};
    use egui::Shape;

    #[test]
    fn the_current_folder_is_not_offered_as_a_destination() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);

        let mut vault = Vault::new("Personal");
        let mut a = Folder::new("Alpha", None, 0);
        a.id = "a".to_owned();
        let mut b = Folder::new("Beta", None, 0);
        b.id = "b".to_owned();
        vault.folders = vec![a, b];

        let tree = tree(&vault);
        let mut draw = |ui: &mut egui::Ui| {
            rows(ui, &icons, &tree, Some("a".to_owned()));
        };
        shot::frame(&ctx, egui::vec2(240.0, 200.0), Vec::new(), &mut draw);
        let shapes = shot::frame(&ctx, egui::vec2(240.0, 200.0), Vec::new(), &mut draw);
        let said: Vec<_> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(
            !said.iter().any(|w| w == "Alpha"),
            "current folder offered: {said:?}"
        );
        assert!(
            said.iter().any(|w| w == "Beta"),
            "other folder missing: {said:?}"
        );
    }
}
