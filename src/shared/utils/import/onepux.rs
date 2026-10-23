//! 1Password's `.1pux`: a zip whose `export.data` is JSON — accounts, their vaults,
//! and the items inside. Only the shapes this app has are mapped: a login, a credit
//! card, a secure note, an identity; anything else is kept as a note with whatever
//! text the item carries, so a file never imports empty.

use super::{field, first, text, Bundle, DEPTH};
use crate::shared::i18n::t;
use crate::vault::model::{now_secs, Card, Field, Identity, Login, Note, Otp};
use serde_json::Value;
use std::io::Read;

/// The largest `export.data` this will look at, inflated.
const EXPORT_DATA: u64 = 64 * 1024 * 1024;

pub fn parse(bytes: &[u8]) -> Result<Bundle, String> {
    let data = export_data(bytes)?;
    let root: Value =
        serde_json::from_slice(&data).map_err(|_| t("that file is not valid JSON").to_owned())?;
    let mut bundle = Bundle::default();

    // The two envelopes 1Password has written: every account's vaults, and a bare
    // list of items.
    let items: Vec<&Value> = root
        .get("accounts")
        .and_then(Value::as_array)
        .map(|accounts| {
            accounts
                .iter()
                .flat_map(|account| {
                    account
                        .get("vaults")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                })
                .flat_map(|vault| {
                    vault
                        .get("items")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_else(|| {
            root.get("items")
                .and_then(Value::as_array)
                .map(|list| list.iter().collect())
                .unwrap_or_default()
        });
    if items.is_empty() {
        return Err(t("this 1Password export has no items").to_owned());
    }

    for item in items {
        land(&mut bundle, item);
    }
    Ok(bundle)
}

/// The `export.data` inside the zip, inflated with a ceiling: a zip is untrusted and
/// a small one can inflate to gigabytes.
fn export_data(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| t("that file is not a zip").to_owned())?;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|_| t("that file is not a zip").to_owned())?;
        match file.name() {
            Ok(name) if name.ends_with("export.data") => {}
            _ => continue,
        }
        let mut data = Vec::new();
        file.by_ref()
            .take(EXPORT_DATA)
            .read_to_end(&mut data)
            .map_err(|_| t("that 1Password export cannot be read").to_owned())?;
        return Ok(data);
    }
    Err(t("this 1Password export carries no export.data").to_owned())
}

fn land(bundle: &mut Bundle, item: &Value) {
    let now = now_secs();
    let name = text(first(item, &["title"]));
    let details = item.get("details").cloned().unwrap_or(Value::Null);
    let overview = item.get("overview").cloned().unwrap_or(Value::Null);
    let notes = {
        let said = text(first(item, &["note"]));
        if said.is_empty() {
            text(first(&details, &["notesPlain"]))
        } else {
            said
        }
    };
    let category = text(first(item, &["category"]));
    let sections = sections(&details);
    let path = folder(item);

    match category.as_str() {
        // Login and Password.
        "001" | "005" => {
            let mut it = Login::new(now);
            it.name = name.clone();
            it.notes = zeroize::Zeroizing::new(notes.clone());
            let mut fields = sections;
            for entry in login_fields(&details) {
                match entry.0.as_str() {
                    "username" => it.username = entry.1,
                    "password" => it.password = zeroize::Zeroizing::new(entry.1),
                    _ => fields.push(field(&entry.0, &entry.1, false)),
                }
            }
            it.url = text(first(&overview, &["url", "URL"]));
            if it.url.is_empty() {
                if let Some(url) = fields
                    .iter()
                    .find(|f| f.label.to_lowercase().contains("url"))
                {
                    it.url = url.value.to_string();
                }
            }
            it.fields = fields;
            bundle.logins.push(it);

            let seed = text(first(&details, &["totp"]));
            if !seed.is_empty() {
                let mut otp = Otp::new(now);
                otp.name = name.clone();
                otp.secret = zeroize::Zeroizing::new(seed);
                bundle.otps.push(otp);
            }
        }
        // Credit Card.
        "002" => {
            let mut it = Card::new(now);
            it.name = name.clone();
            it.notes = zeroize::Zeroizing::new(notes);
            it.number = zeroize::Zeroizing::new(text(first(&details, &["cardNumber", "number"])));
            it.holder = text(first(&details, &["cardholderName", "cardholder"]));
            it.expiry = text(first(&details, &["expiryDate", "expiry"]));
            let mut all = sections;
            let code = text(first(&details, &["verificationNumber", "code", "cvv"]));
            if !code.is_empty() {
                all.push(field("CVV", &code, true));
            }
            it.fields = all;
            bundle.cards.push(it);
        }
        // Secure Note.
        "003" => {
            let mut it = Note::new(now);
            it.name = name;
            it.body = zeroize::Zeroizing::new(notes);
            bundle.notes.push((path, it));
        }
        // Identity and the documents that are one: a passport, a licence.
        "004" | "010" | "013" => {
            let mut it = Identity::new(now);
            it.name = name.clone();
            it.notes = zeroize::Zeroizing::new(notes);
            it.fields = sections;
            bundle.identities.push(it);
        }
        // Everything else 1Password has (a bank account, a reward, a server) is kept
        // as a note with its own text, so nothing is silently dropped.
        _ => {
            let mut it = Note::new(now);
            it.name = name;
            let mut body = notes;
            for field in sections {
                if field.value.is_empty() {
                    continue;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&format!("{}: {}", field.label, field.value.as_str()));
            }
            it.body = zeroize::Zeroizing::new(body);
            bundle.notes.push((path, it));
        }
    }
}

/// The folder an item names, one level per segment and no deeper than an import goes.
fn folder(item: &Value) -> String {
    let mut path: Vec<String> = Vec::new();
    let mut here = item.get("folder").filter(|it| it.is_object());
    while let Some(at) = here {
        if path.len() == DEPTH {
            break;
        }
        let name = text(first(at, &["title", "name"]));
        if !name.is_empty() {
            path.insert(0, name);
        }
        here = at.get("folder").filter(|it| it.is_object());
    }
    path.join("/")
}

/// `details.loginFields[]`: what 1Password calls each login column.
fn login_fields(details: &Value) -> Vec<(String, String)> {
    details
        .get("loginFields")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|it| {
                    let value = text(first(it, &["value"]));
                    if value.is_empty() {
                        return None;
                    }
                    let designation = text(first(it, &["designation", "name"])).to_lowercase();
                    Some((designation, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `details.sections[].fields[]`: the custom rows, where a PIN or a code lives. A
/// field whose designation or label says password is marked secret.
fn sections(details: &Value) -> Vec<Field> {
    details
        .get("sections")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .flat_map(|section| {
                    section
                        .get("fields")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                })
                .filter_map(|it| {
                    let label = text(first(it, &["title", "name"]));
                    let value = text(first(it, &["value"]));
                    if value.is_empty() || label.is_empty() {
                        return None;
                    }
                    let designation = text(
                        first(it, &["inputTraits"])
                            .and_then(|traits| traits.get("designation"))
                            .or(Some(it.get("designation").unwrap_or(&Value::Null))),
                    );
                    let secret = designation.eq_ignore_ascii_case("password")
                        || label.to_lowercase().contains("password")
                        || label.to_lowercase().contains("pin");
                    Some(field(&label, &value, secret))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const EXPORT: &str = r#"{"accounts": [{"vaults": [{"items": [
      {"title": "GitHub", "category": "001", "note": "",
       "overview": {"url": "https://github.com"},
       "details": {"loginFields": [
          {"designation": "username", "value": "octocat"},
          {"designation": "password", "value": "hunter2"}],
         "sections": [{"title": "Extra", "fields": [{"title": "PIN", "value": "1234"}]}]}},
      {"title": "Visa", "category": "002",
       "details": {"cardNumber": "4111", "cardholderName": "Ana", "expiryDate": "04/29", "verificationNumber": "123"}},
      {"title": "Ideas", "category": "003", "note": "one", "folder": {"title": "Personal"}},
      {"title": "Passport", "category": "013",
       "details": {"sections": [{"fields": [{"title": "number", "value": "X1"}]}]}}
    ]}]}]}"#;

    fn zip_of(name: &str, body: &str) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn an_export_becomes_the_shapes_it_carries() {
        let bytes = zip_of("export/export.data", EXPORT);
        let b = parse(&bytes).unwrap();
        assert_eq!(b.logins.len(), 1);
        assert_eq!(b.logins[0].username, "octocat");
        assert_eq!(b.logins[0].password.as_str(), "hunter2");
        assert_eq!(b.logins[0].url, "https://github.com");
        assert_eq!(b.logins[0].fields[0].label, "PIN");
        assert!(b.logins[0].fields[0].secret, "a PIN is hidden");

        assert_eq!(b.cards[0].number.as_str(), "4111");
        assert_eq!(b.cards[0].expiry, "04/29");
        assert!(b.cards[0].fields.iter().any(|f| f.label == "CVV"));

        assert_eq!(b.notes[0].0, "Personal", "the folder named it");
        assert_eq!(b.notes[0].1.name, "Ideas");
        assert_eq!(b.identities[0].fields[0].label, "number");
    }

    #[test]
    fn a_zip_without_export_data_is_refused() {
        let bytes = zip_of("other.txt", "nothing");
        assert!(parse(&bytes).is_err());
        assert!(parse(b"not a zip").is_err());
    }
}
