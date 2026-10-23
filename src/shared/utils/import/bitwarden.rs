//! Bitwarden's plain JSON export (`items` with a numeric `type`). An encrypted
//! export is refused by name: it is a different file, not a broken one.

use super::{field, first, text, Bundle, DEPTH};
use crate::shared::i18n::t;
use crate::vault::model::{now_secs, Card, Field, Identity, Login, Note, Otp};
use serde_json::Value;
use std::collections::HashMap;

pub fn parse(bytes: &[u8]) -> Result<Bundle, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| t("that file is not valid JSON").to_owned())?;
    if root.get("encrypted").and_then(Value::as_bool) == Some(true) {
        return Err(t("this is an encrypted Bitwarden export — export it unencrypted").to_owned());
    }
    let Some(items) = root.get("items").and_then(Value::as_array) else {
        return Err(t("this is not a Bitwarden export").to_owned());
    };
    // A folder in Bitwarden is one level deep, and it only places a note here: a
    // login, a card and an identity have no folder in this box.
    let folders: HashMap<&str, &str> = root
        .get("folders")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|it| Some((it.get("id")?.as_str()?, it.get("name")?.as_str()?.trim())))
                .collect()
        })
        .unwrap_or_default();

    let mut bundle = Bundle::default();
    for item in items {
        let kind = item.get("type").and_then(Value::as_u64).unwrap_or(0);
        let name = text(item.get("name"));
        let notes = text(item.get("notes"));
        let fields = fields(item.get("fields"));
        match kind {
            1 => {
                let login = item.get("login").cloned().unwrap_or(Value::Null);
                let mut it = Login::new(now_secs());
                it.name = name.clone();
                it.username = text(first(&login, &["username"]));
                it.password = zeroize::Zeroizing::new(text(first(&login, &["password"])));
                it.notes = zeroize::Zeroizing::new(notes.clone());
                let uris: Vec<String> = login
                    .get("uris")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|uri| text(first(uri, &["uri"])))
                            .filter(|uri| !uri.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some((main, rest)) = uris.split_first() {
                    it.url = main.clone();
                    it.extra_urls = rest
                        .iter()
                        .map(|url| crate::vault::matching::Uri {
                            url: url.clone(),
                            rule: crate::vault::matching::Rule::Domain,
                        })
                        .collect();
                }
                it.fields = fields;
                bundle.logins.push(it);

                let seed = text(first(&login, &["totp"]));
                if !seed.is_empty() {
                    bundle.otps.push(otp(&name, &seed));
                }
            }
            2 => {
                let mut it = Note::new(now_secs());
                it.name = name.clone();
                it.body = zeroize::Zeroizing::new(notes.clone());
                let path = item
                    .get("folderId")
                    .and_then(Value::as_str)
                    .and_then(|id| folders.get(id))
                    .map(|name| cap(name))
                    .unwrap_or_default();
                bundle.notes.push((path, it));
            }
            3 => {
                let card = item.get("card").cloned().unwrap_or(Value::Null);
                let mut it = Card::new(now_secs());
                it.name = name.clone();
                it.notes = zeroize::Zeroizing::new(notes.clone());
                it.holder = text(first(&card, &["cardholderName"]));
                it.number = zeroize::Zeroizing::new(text(first(&card, &["number"])));
                it.expiry = expiry(&card);
                let mut all = fields;
                let code = text(first(&card, &["code"]));
                if !code.is_empty() {
                    all.push(field("CVV", &code, true));
                }
                it.fields = all;
                bundle.cards.push(it);
            }
            4 => {
                let mut it = Identity::new(now_secs());
                it.name = name.clone();
                it.notes = zeroize::Zeroizing::new(notes.clone());
                let mut all: Vec<Field> = vec![];
                if let Some(map) = item.get("identity").and_then(Value::as_object) {
                    for (label, value) in map {
                        let value = text(Some(value));
                        if !value.is_empty() {
                            all.push(field(label, &value, false));
                        }
                    }
                }
                all.extend(fields);
                it.fields = all;
                bundle.identities.push(it);
            }
            _ => {}
        }
    }
    Ok(bundle)
}

fn otp(name: &str, seed: &str) -> Otp {
    let mut it = Otp::new(now_secs());
    it.name = name.to_owned();
    it.secret = zeroize::Zeroizing::new(seed.to_owned());
    it
}

/// `fields: [{name, value, type}]`, where type 1 is a hidden field and type 2 is a
/// boolean. Only the hidden one is a secret.
fn fields(value: Option<&Value>) -> Vec<Field> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|it| {
                    let label = text(first(it, &["name"]));
                    let value = text(first(it, &["value"]));
                    if value.is_empty() {
                        return None;
                    }
                    let secret = first(it, &["type"]).and_then(Value::as_u64) == Some(1);
                    Some(field(&label, &value, secret))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `expMonth` and `expYear` as the card prints them; a year already written as four
/// digits stays four, and a two-digit one is padded.
fn expiry(card: &Value) -> String {
    let month = text(first(card, &["expMonth"]));
    let year = text(first(card, &["expYear"]));
    if month.is_empty() || year.is_empty() {
        return String::new();
    }
    let month: u32 = month.parse().unwrap_or(0);
    let year: u32 = year.parse().unwrap_or(0);
    let year = match year {
        0..=99 => 2000 + year,
        _ => year,
    };
    format!("{month:02}/{:02}", year % 100)
}

/// A folder path from a name, cut to the depth an import places.
fn cap(name: &str) -> String {
    name.split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(DEPTH)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = r#"{
      "encrypted": false,
      "folders": [{"id": "f1", "name": "Personal"}],
      "items": [
        {"type": 1, "name": "GitHub", "notes": "ssh key", "folderId": null,
         "fields": [{"name": "Recovery", "value": "abc", "type": 1}],
         "login": {"username": "octocat", "password": "hunter2",
                   "uris": [{"uri": "https://github.com"}, {"uri": "https://gist.github.com"}],
                   "totp": "JBSWY3DPEHPK3PXP"}},
        {"type": 2, "name": "Ideas", "notes": "one\ntwo", "folderId": "f1"},
        {"type": 3, "name": "Visa", "notes": "", "card": {"cardholderName": "Ana", "number": "4111", "expMonth": "4", "expYear": "2029", "code": "123"}},
        {"type": 4, "name": "Ana", "notes": "passport", "identity": {"passportNumber": "X1", "firstName": ""}}
      ]
    }"#;

    #[test]
    fn every_shape_lands_in_its_own_collection() {
        let b = parse(EXPORT.as_bytes()).unwrap();
        assert_eq!(
            b.count(),
            5,
            "a login, its seed, a note, a card, an identity"
        );
        assert_eq!(b.logins.len(), 1);
        assert_eq!(b.logins[0].name, "GitHub");
        assert_eq!(b.logins[0].username, "octocat");
        assert_eq!(b.logins[0].url, "https://github.com");
        assert_eq!(b.logins[0].extra_urls.len(), 1);
        assert!(b.logins[0].fields[0].secret, "a hidden field is a secret");
        assert_eq!(b.otps[0].name, "GitHub");
        assert_eq!(b.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");

        assert_eq!(b.notes[0].0, "Personal", "the folder named it");
        assert_eq!(b.notes[0].1.body.as_str(), "one\ntwo");

        assert_eq!(b.cards[0].number.as_str(), "4111");
        assert_eq!(b.cards[0].holder, "Ana");
        assert_eq!(b.cards[0].expiry, "04/29");
        assert!(b.cards[0]
            .fields
            .iter()
            .any(|it| it.label == "CVV" && it.secret));

        assert_eq!(
            b.identities[0].fields.len(),
            1,
            "an empty value is not a field"
        );
        assert_eq!(b.identities[0].fields[0].label, "passportNumber");
    }

    #[test]
    fn an_encrypted_export_is_refused_by_name() {
        let err = parse(br#"{"encrypted": true, "items": []}"#).unwrap_err();
        assert!(err.contains("encrypted"), "{err}");
        assert!(parse(br#"{"data": []}"#).is_err(), "not a Bitwarden export");
    }
}
