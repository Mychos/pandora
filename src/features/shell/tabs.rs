//! Several vaults unlocked at once: the one on screen is `Boxes::session`, the
//! rest wait in `Boxes::tabs` in strip order.

use crate::app::{App, Screen, Session};
use crate::features::boxes::model::Boxes;
use crate::vault::model::Vault;
use std::path::Path;
use zeroize::Zeroize;

impl Boxes {
    /// Every open box, the one on screen first: the browser lists them and names the
    /// one it reads from. A parked tab keeps the box's name, so a listing costs no
    /// decrypting.
    pub fn open(&self) -> Vec<(&Path, &str)> {
        self.session
            .iter()
            .chain(self.tabs.iter())
            .map(|s| (s.path.as_path(), s.vault.name.as_str()))
            .collect()
    }

    /// The box a browser request reads: the one it picked while that box is still
    /// open, else the one on screen. A parked tab is loaded for the answer — it holds
    /// the key already, so no password is asked — and the caller parks it again.
    pub fn pick(&mut self, path: Option<&Path>) -> Result<Option<&mut Session>, String> {
        let Some(path) = path else {
            return Ok(self.session.as_mut());
        };
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.path.as_path() == path)
        {
            return Ok(self.session.as_mut());
        }
        if let Some(at) = self.tabs.iter().position(|s| s.path.as_path() == path) {
            self.tabs[at].load_parked()?;
            return Ok(Some(&mut self.tabs[at]));
        }
        Ok(self.session.as_mut())
    }

    /// Every open box's vault, for a read that spans them: the one on screen, then
    /// the strip. A parked tab stays parked once `f` is done with it — it is decrypted
    /// for the look and put back, so a page can match what every box holds.
    pub fn scan<R>(&mut self, mut f: impl FnMut(&Path, &Vault) -> R) -> Vec<R> {
        let mut out = Vec::new();
        for session in self.session.iter_mut().chain(self.tabs.iter_mut()) {
            let parked = session.parked;
            if parked && session.load_parked().is_err() {
                continue;
            }
            out.push(f(session.path.as_path(), &session.vault));
            if parked {
                session.park();
            }
        }
        out
    }

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
        if i == self.boxes.active {
            self.screen = Screen::Main;
            return;
        }
        // Load the tab we are going to before parking the one on screen: a box
        // that cannot be read leaves the current one untouched.
        let at = if i < self.boxes.active { i } else { i - 1 };
        let mut all = std::mem::take(&mut self.boxes.tabs);
        let mut incoming = all.remove(at);
        if let Err(e) = incoming.load_parked() {
            all.insert(at, incoming);
            self.boxes.tabs = all;
            self.shell.fail(crate::shared::i18n::tf(
                "could not open the box: {error}",
                &[("error", &e)],
            ));
            return;
        }
        self.boxes.flush(&mut self.shell);
        let Some(mut current) = self.boxes.session.take() else {
            self.boxes.session = Some(incoming);
            self.boxes.tabs = all;
            self.boxes.active = i;
            self.screen = Screen::Main;
            return;
        };
        current.park();
        let insert_at = if i < self.boxes.active {
            self.boxes.active - 1
        } else {
            self.boxes.active
        };
        all.insert(insert_at.min(all.len()), current);
        self.boxes.session = Some(incoming);
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
        // The closed box's player, page, picture and palette belong to it, not
        // to whatever tab slides into the slot (or to the sources screen).
        self.preview = Default::default();
        self.find = Default::default();
        if self.boxes.tabs.is_empty() {
            self.boxes.active = 0;
            self.screen = self.auth.landing();
            self.forget_closed();
            return;
        }
        // the tab that slid into this slot, or the one before it when the last was closed
        let next = i.min(self.boxes.tabs.len() - 1);
        let mut incoming = self.boxes.tabs.remove(next);
        if let Err(e) = incoming.load_parked() {
            self.shell.fail(crate::shared::i18n::tf(
                "could not open the box: {error}",
                &[("error", &e)],
            ));
        }
        // A box that cannot be read keeps its tab, parked and empty: the error
        // says why, and `write` refuses to put that placeholder over the file.
        self.boxes.session = Some(incoming);
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

    /// A parked tab keeps the key and the path but not the payload; switching
    /// back reads the file with that key and restores the edits that were written
    /// before it left the screen.
    #[test]
    fn a_parked_tab_drops_its_vault_and_loads_it_back() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("Personal");
        {
            let s = app.boxes.session.as_mut().unwrap();
            let mut login = crate::vault::model::Login::new(1);
            login.name = "only in personal".to_owned();
            s.vault.logins.push(login);
            s.mark_dirty();
        }
        app.boxes.persist(&mut app.shell);

        app.open("Work");
        let parked = &app.boxes.tabs[0];
        assert!(parked.parked, "the box left the screen parked");
        assert!(
            parked.vault.logins.is_empty(),
            "the decrypted payload is not in the tab"
        );
        assert_eq!(
            crate::vault::store::load(&parked.path, &parked.creds)
                .unwrap()
                .logins[0]
                .name,
            "only in personal",
            "the edit was written before it was parked"
        );

        app.switch(0);
        let s = app.boxes.session.as_ref().unwrap();
        assert!(!s.parked);
        assert_eq!(s.vault.logins[0].name, "only in personal");
    }

    /// Closing the box on screen also closes its palette: the rows belonged to
    /// the box that is gone, and the next tab starts with a clean finder.
    #[test]
    fn closing_the_active_box_closes_its_palette() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("Personal");
        app.find.open = true;
        app.find.query = "only in personal".to_owned();
        app.close(0);
        assert!(!app.find.open, "the palette went with the box");
        assert!(app.find.query.is_empty());
    }
}
