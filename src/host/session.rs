//! The unlocked box the host holds: derived key, vault, and the reads the extension
//! asks for, through the same `vault::store` as the app.

use crate::bridge::protocol::{Candidate, Response};
use crate::bridge::serve;
use crate::vault::model::Vault;
use crate::vault::store::{self, Credentials};
use std::path::PathBuf;

pub struct Session {
    path: PathBuf,
    creds: Credentials,
    vault: Vault,
}

impl Session {
    /// The derived key lives for the process and never crosses back to the browser.
    pub fn unlock(path: PathBuf, password: &str) -> Result<Self, String> {
        // A key on the desk answers for a box that enrolled one; the password alone
        // still opens every box that did not.
        let key = crate::shared::yubikey::Usb;
        let factor = crate::shared::yubikey::present().then_some(&key as &dyn store::Factor);
        let (creds, vault) = store::open_with(&path, password, None, factor)?;
        Ok(Self { path, creds, vault })
    }

    pub fn name(&self) -> &str {
        &self.vault.name
    }

    pub fn count(&self) -> usize {
        self.vault.logins.len()
    }

    /// Re-read and decrypt with the cached key: the app may have written since.
    pub fn refresh(&mut self) -> Result<(), String> {
        self.vault = store::load(&self.path, &self.creds)?;
        Ok(())
    }

    /// The box itself, for the shared reader both peers answer from.
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Re-read first, so an edit made in the app while the panel was open is not lost.
    pub fn create(
        &mut self,
        name: String,
        username: String,
        password: &str,
        url: String,
    ) -> Result<Candidate, String> {
        self.refresh()?;
        let entry = serve::create_login(&mut self.vault, name, username, password, url);
        self.save()?;
        Ok(entry)
    }

    /// Same re-read first, so the app's other edits survive.
    pub fn update(
        &mut self,
        id: &str,
        username: Option<String>,
        password: Option<&str>,
    ) -> Result<Candidate, String> {
        self.refresh()?;
        let entry = serve::update_login(&mut self.vault, id, username, password)
            .ok_or_else(|| "no such entry".to_owned())?;
        self.save()?;
        Ok(entry)
    }

    /// A new credential for the site, minted here and saved into the box.
    pub fn passkey_create(&mut self, rp_id: &str, user_name: &str) -> Response {
        self.refresh_response(|session| {
            let response = crate::host::passkeys::create_in(&mut session.vault, rp_id, user_name);
            (response, true)
        })
    }

    /// One assertion for the credentials a page already knows; the counter moves with it.
    pub fn passkey_assert(
        &mut self,
        rp_id: &str,
        credential_id: &[u8],
        client_data: &[u8],
    ) -> Response {
        self.refresh_response(|session| {
            let response = crate::host::passkeys::assert_in(
                &mut session.vault,
                rp_id,
                credential_id,
                client_data,
            );
            (response, true)
        })
    }

    /// Re-read first, so an edit made in the app is not lost, then run the change and
    /// write the box when the change says it changed something.
    fn refresh_response<F>(&mut self, change: F) -> Response
    where
        F: FnOnce(&mut Self) -> (Response, bool),
    {
        if let Err(message) = self.refresh() {
            return Response::Error { message };
        }
        let (response, touched) = change(self);
        if !touched || matches!(response, Response::Error { .. }) {
            return response;
        }
        match self.save() {
            Ok(()) => response,
            Err(message) => Response::Error { message },
        }
    }

    fn save(&mut self) -> Result<(), String> {
        store::save(&self.path, &mut self.vault, &self.creds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::Login;
    use zeroize::Zeroizing;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("pandora-host-{}.pandora", uuid::Uuid::new_v4()))
    }

    /// entries: (name, username, url)
    fn session(entries: &[(&str, &str, &str)]) -> (Session, PathBuf) {
        let path = temp();
        let (creds, mut vault) = store::create(&path, "Box", "pw", None).unwrap();
        for (name, username, url) in entries {
            let mut item = Login::new(1);
            item.name = (*name).to_owned();
            item.username = (*username).to_owned();
            item.password = Zeroizing::new("secret".to_owned());
            item.url = (*url).to_owned();
            vault.logins.push(item);
        }
        store::save(&path, &mut vault, &creds).unwrap();
        (Session::unlock(path.clone(), "pw").unwrap(), path)
    }

    /// A credential minted through the host is in the file, not only in memory, and
    /// the assertion's counter goes there too.
    #[test]
    fn a_passkey_is_written_and_its_counter_is_kept() {
        let (mut s, path) = session(&[]);
        let answer = s.passkey_create("example.com", "ana@example.com");
        let crate::bridge::protocol::Response::PasskeyCreated { credential_id, .. } = answer else {
            panic!("a credential comes back: {answer:?}");
        };
        let id = crate::vault::codec::decode(&credential_id).unwrap();

        let answer = s.passkey_assert("example.com", &id, br#"{"challenge":"AAAA"}"#);
        assert!(
            matches!(
                answer,
                crate::bridge::protocol::Response::PasskeyAssertion { counter: 1, .. }
            ),
            "{answer:?}"
        );

        let again = Session::unlock(path.clone(), "pw").unwrap();
        assert_eq!(again.vault().passkeys.len(), 1, "the box on disk has it");
        assert_eq!(again.vault().passkeys[0].counter, 1);
        assert_eq!(
            again.vault().passkeys[0].name,
            "ana@example.com",
            "the account name is what the record is called"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn matches_by_site_and_its_subdomains_only() {
        let (s, path) = session(&[
            ("GitHub", "octocat", "https://github.com/login"),
            ("Mail", "me", "example.com"),
        ]);
        assert_eq!(serve::matches(s.vault(), "github.com").len(), 1);
        assert_eq!(
            serve::matches(s.vault(), "gist.github.com").len(),
            1,
            "a subdomain still matches"
        );
        assert_eq!(
            serve::matches(s.vault(), "github.com.evil.com").len(),
            0,
            "not a real parent"
        );
        assert_eq!(
            serve::matches(s.vault(), "example.com:8443").len(),
            1,
            "port is dropped"
        );
        assert_eq!(serve::matches(s.vault(), "nothing.com").len(), 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_entry_without_a_url_never_matches() {
        let (s, path) = session(&[("No URL", "u", "")]);
        assert!(serve::matches(s.vault(), "github.com").is_empty());
        assert_eq!(
            serve::entries(s.vault()).len(),
            1,
            "but the popup still lists it"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_password_leaves_only_through_reveal() {
        let (s, path) = session(&[("GitHub", "octocat", "github.com")]);
        let listed = &serve::matches(s.vault(), "github.com")[0];
        assert_eq!(listed.username, "octocat");

        let (username, password) = serve::reveal(s.vault(), &listed.id).unwrap();
        assert_eq!(username, "octocat");
        assert_eq!(password, "secret");
        assert!(serve::reveal(s.vault(), "no-such-id").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_wrong_password_does_not_unlock() {
        let path = temp();
        store::create(&path, "Box", "right", None).unwrap();
        assert!(Session::unlock(path.clone(), "wrong").is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_save_from_the_panel_keeps_what_the_app_wrote() {
        let path = temp();
        let (creds, mut vault) = store::create(&path, "Box", "pw", None).unwrap();
        vault.logins.push(Login::new(1));
        store::save(&path, &mut vault, &creds).unwrap();
        let mut session = Session::unlock(path.clone(), "pw").unwrap();

        // The write must re-read first, or the app's entry is lost.
        vault.logins.push(Login::new(2));
        store::save(&path, &mut vault, &creds).unwrap();

        session
            .create(
                "New".to_owned(),
                "u".to_owned(),
                "p",
                "example.com".to_owned(),
            )
            .unwrap();

        let (_c, saved) = store::open(&path, "pw", None).unwrap();
        assert_eq!(saved.logins.len(), 3, "the app's entry survived the write");
        assert_eq!(session.count(), 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_update_leaves_the_fields_it_was_not_given() {
        let (mut s, path) = session(&[("GitHub", "octocat", "github.com")]);
        let id = serve::matches(s.vault(), "github.com")[0].id.clone();

        s.update(&id, None, Some("rotated")).unwrap();
        let (username, password) = serve::reveal(s.vault(), &id).unwrap();
        assert_eq!(
            username, "octocat",
            "the username was not sent, so it stays"
        );
        assert_eq!(password, "rotated");
        assert!(s.update("no-such-id", None, Some("x")).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_new_entry_claims_the_page_it_was_saved_from() {
        let (mut s, path) = session(&[]);
        assert!(serve::matches(s.vault(), "example.com").is_empty());

        s.create(
            "Example".to_owned(),
            "eve".to_owned(),
            "secret2",
            "https://example.com/login".to_owned(),
        )
        .unwrap();
        let listed = &serve::matches(s.vault(), "https://example.com/session")[0];
        assert_eq!(listed.username, "eve");
        assert_eq!(listed.uri.as_deref(), Some("https://example.com/login"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn refresh_picks_up_what_the_app_wrote() {
        let path = temp();
        let (creds, mut vault) = store::create(&path, "Box", "pw", None).unwrap();
        let mut session = Session::unlock(path.clone(), "pw").unwrap();
        assert_eq!(session.count(), 0);

        vault.logins.push(Login::new(1));
        store::save(&path, &mut vault, &creds).unwrap();
        session.refresh().unwrap();
        assert_eq!(session.count(), 1, "the cached vault was stale");
        let _ = std::fs::remove_file(path);
    }
}
