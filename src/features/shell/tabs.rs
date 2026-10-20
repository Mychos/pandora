//! Several vaults unlocked at once: the one on screen is `Boxes::session`, the
//! rest wait in `Boxes::tabs` in strip order.

use crate::app::{App, Screen, Session};
use crate::features::boxes::model::Boxes;
use std::path::Path;
use zeroize::Zeroize;

impl Boxes {
    pub(crate) fn position(&self, path: &Path) -> Option<usize> {
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.path.as_path() == path)
        {
            return Some(self.active);
        }
        let i = self.tabs.iter().position(|s| s.path.as_path() == path)?;
        Some(if i < self.active { i } else { i + 1 })
    }

    /// On screen or parked; a worker result lands on the box that asked for it.
    pub(crate) fn find(&mut self, path: &Path) -> Option<&mut Session> {
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.path.as_path() == path)
        {
            return self.session.as_mut();
        }
        self.tabs.iter_mut().find(|s| s.path.as_path() == path)
    }
}

impl App {
    /// Picking the tab already on screen is allowed: that is how the Unlock
    /// screen gets out of the way.
    pub fn switch(&mut self, i: usize) {
        if i != self.boxes.active {
            self.boxes.flush(&mut self.shell);
        }
        let Some(current) = self.boxes.session.take() else {
            return;
        };
        let mut all = std::mem::take(&mut self.boxes.tabs);
        all.insert(self.boxes.active.min(all.len()), current);
        self.boxes.session = Some(all.remove(i));
        self.boxes.tabs = all;
        self.boxes.active = i;
        self.screen = Screen::Main;
    }

    pub fn close(&mut self, i: usize) {
        if i != self.boxes.active {
            // a parked tab: the one on screen keeps both its slot and its key
            let at = if i < self.boxes.active { i } else { i - 1 };
            let removed = self.boxes.tabs.remove(at);
            // A parked box's edits only live in its vault: they go to disk before
            // the key does, exactly as they would from the slot on screen.
            Boxes {
                session: Some(removed),
                ..Default::default()
            }
            .flush(&mut self.shell);
            if i < self.boxes.active {
                self.boxes.active -= 1;
            }
            self.forget_closed();
            return;
        }
        self.boxes.flush(&mut self.shell);
        self.boxes.session = None;
        if self.boxes.tabs.is_empty() {
            self.boxes.active = 0;
            self.screen = self.auth.landing();
            self.forget_closed();
            return;
        }
        // the tab that slid into this slot, or the one before it when the last was closed
        let next = i.min(self.boxes.tabs.len() - 1);
        self.boxes.session = Some(self.boxes.tabs.remove(next));
        self.boxes.active = next;
        self.screen = Screen::Main;
        self.forget_closed();
    }

    pub fn add(&mut self) {
        self.auth.unlock_password.zeroize();
        self.auth.unlock_path.clear();
        self.auth.unlock_keyfile.clear();
        self.auth.recent_search.clear();
        self.screen = Screen::Sources;
    }

    /// Ctrl+Tab (Ctrl+Shift+Tab): the next (previous) box in the strip.
    pub fn rotate(&mut self, forward: bool) {
        if self.boxes.session.is_none() {
            return;
        }
        let total = self.boxes.tabs.len() + 1;
        if total < 2 {
            return;
        }
        let step = if forward { 1 } else { total - 1 };
        self.switch((self.boxes.active + step) % total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;

    fn strip(app: &App) -> Vec<String> {
        let mut all: Vec<String> = app
            .boxes
            .tabs
            .iter()
            .map(|s| s.vault.name.clone())
            .collect();
        if let Some(s) = app.boxes.session.as_ref() {
            all.insert(app.boxes.active, s.vault.name.clone());
        }
        all
    }

    #[test]
    fn vaults_open_side_by_side_and_close_in_order() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);

        app.open("Personal");
        assert_eq!(strip(&app), ["Personal"]);
        assert_eq!(app.boxes.active, 0);

        app.open("Work");
        assert_eq!(strip(&app), ["Personal", "Work"]);
        assert_eq!(
            app.boxes.active, 1,
            "the vault just unlocked is the one on screen"
        );

        app.switch(0);
        assert_eq!(strip(&app), ["Personal", "Work"]);
        assert_eq!(app.boxes.active, 0);
        assert!(app.screen == Screen::Main);

        app.close(1);
        assert_eq!(strip(&app), ["Personal"]);
        assert_eq!(app.boxes.active, 0);

        app.close(0);
        assert!(app.boxes.session.is_none() && app.boxes.tabs.is_empty());
        assert!(
            app.screen == Screen::Sources,
            "the last close lands on the sources"
        );
    }

    #[test]
    fn the_same_file_never_opens_twice() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("Personal");
        app.open("Work");

        app.open("Personal");
        assert_eq!(strip(&app), ["Personal", "Work"]);
        assert_eq!(app.boxes.active, 0);
    }
}
