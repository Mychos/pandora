//! KeePass's XML export (`KeePassFile → Root → Group → Entry`), read event by event:
//! the file can be large, and only the entries are wanted. A group organises
//! logins in KeePass, and a login has no folder in this box, so groups name nothing
//! here — what a user kept in them stays in the records themselves.

use super::Bundle;
use crate::shared::i18n::t;
use crate::vault::model::{now_secs, Field, Login, Otp};
use quick_xml::events::Event;
use quick_xml::Reader;

/// The `<String>` keys that are a login's own columns; everything else is a field.
const KNOWN: [&str; 5] = ["Title", "UserName", "Password", "URL", "Notes"];

pub fn parse(bytes: &[u8]) -> Result<Bundle, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| t("this KeePass export is not UTF-8").to_owned())?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut bundle = Bundle::default();
    // One entry's strings while it is open, and whether the history is what is being
    // read: a history entry is a previous version and not a record of the box.
    let mut entry: Option<Vec<(String, String, bool)>> = None;
    let mut history: usize = 0;
    let mut key: Option<String> = None;
    let mut value: Option<(String, bool)> = None;
    let mut spot: Option<Spot> = None;
    let mut found = 0usize;

    loop {
        let event = reader
            .read_event()
            .map_err(|_| t("that file is not valid XML").to_owned())?;
        match event {
            Event::Start(start) => match start.name().as_ref() {
                b"Group" if entry.is_none() => {}
                // A history entry is a previous version of the one above it, not a
                // record of the box: it is read as nothing at all.
                b"Entry" if history == 0 => entry = Some(Vec::new()),
                b"History" => history += 1,
                b"Key" if entry.is_some() => spot = Some(Spot::Key),
                b"Value" if entry.is_some() => {
                    let protected = start
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == b"Protected" && a.value.as_ref() == b"True");
                    value = Some((String::new(), protected));
                    spot = Some(Spot::Value);
                }
                _ => {}
            },
            Event::Empty(start) if start.name().as_ref() == b"Entry" => {
                // An entry with nothing in it is still not a record.
            }
            Event::Text(text) => {
                let said = text.decode().unwrap_or_default().into_owned();
                match spot {
                    Some(Spot::Key) => key.get_or_insert_with(String::new).push_str(&said),
                    Some(Spot::Value) => {
                        if let Some((value, _)) = value.as_mut() {
                            value.push_str(&said);
                        }
                    }
                    None => {}
                }
            }
            Event::End(end) => match end.name().as_ref() {
                b"Key" => spot = None,
                b"Value" => {
                    spot = None;
                    if let (Some(entry), Some(key), Some((value, protected))) =
                        (entry.as_mut(), key.take(), value.take())
                    {
                        entry.push((key, value, protected));
                    }
                }
                b"Entry" if history == 0 => {
                    if let Some(strings) = entry.take() {
                        if !strings.is_empty() {
                            land(&mut bundle, &strings);
                            found += 1;
                        }
                    }
                }
                b"History" => history = history.saturating_sub(1),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    if found == 0 {
        return Err(t("this KeePass export has no entries").to_owned());
    }
    Ok(bundle)
}

enum Spot {
    Key,
    Value,
}

/// One entry's strings as a login, and whatever asked to be a seed as one.
fn land(bundle: &mut Bundle, strings: &[(String, String, bool)]) {
    let read = |name: &str| {
        strings
            .iter()
            .find(|(key, _, _)| key == name)
            .map(|(_, value, _)| value.clone())
            .unwrap_or_default()
    };
    let now = now_secs();
    let name = read("Title");
    let name = if name.trim().is_empty() {
        "Untitled".to_owned()
    } else {
        name
    };

    let mut login = Login::new(now);
    login.name = name.clone();
    login.username = read("UserName");
    login.password = zeroize::Zeroizing::new(read("Password"));
    login.url = read("URL");
    login.notes = zeroize::Zeroizing::new(read("Notes"));

    let mut seed = None;
    let mut fields: Vec<Field> = Vec::new();
    for (key, value, protected) in strings {
        // The first of the two seeds this app knows is the one that is used.
        if key.eq_ignore_ascii_case("otp") || key.eq_ignore_ascii_case("totp") {
            if !value.is_empty() {
                seed = Some(value.clone());
            }
            continue;
        }
        if KNOWN.contains(&key.as_str()) || value.trim().is_empty() {
            continue;
        }
        fields.push(Field::new(key, value, *protected));
    }
    login.fields = fields;
    bundle.logins.push(login);

    if let Some(seed) = seed {
        let mut otp = Otp::new(now);
        otp.name = name;
        otp.secret = zeroize::Zeroizing::new(seed);
        bundle.otps.push(otp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<KeePassFile>
  <Root>
    <Group>
      <Name>Work</Name>
      <Entry>
        <String><Key>Title</Key><Value>GitHub</Value></String>
        <String><Key>UserName</Key><Value>octocat</Value></String>
        <String><Key>Password</Key><Value Protected="True">hunter2</Value></String>
        <String><Key>URL</Key><Value>https://github.com</Value></String>
        <String><Key>Notes</Key><Value>ssh key</Value></String>
        <String><Key>otp</Key><Value>otpauth://totp/ACME?secret=JBSWY3DPEHPK3PXP</Value></String>
        <String><Key>Recovery</Key><Value Protected="True">abc-123</Value></String>
        <History>
          <Entry>
            <String><Key>Title</Key><Value>GitHub old</Value></String>
            <String><Key>Password</Key><Value>older</Value></String>
          </Entry>
        </History>
      </Entry>
      <Entry>
        <String><Key>Title</Key><Value>Bank</Value></String>
        <String><Key>Password</Key><Value>pw</Value></String>
      </Entry>
    </Group>
  </Root>
</KeePassFile>"#;

    #[test]
    fn entries_become_logins_and_a_seed_becomes_a_code() {
        let b = parse(EXPORT.as_bytes()).unwrap();
        assert_eq!(b.logins.len(), 2, "a history entry is not a record");
        assert_eq!(b.logins[0].name, "GitHub");
        assert_eq!(b.logins[0].password.as_str(), "hunter2");
        assert_eq!(b.logins[0].notes.as_str(), "ssh key");
        assert_eq!(
            b.logins[0].fields.len(),
            1,
            "the seed and the columns are not fields"
        );
        assert_eq!(b.logins[0].fields[0].label, "Recovery");
        assert!(
            b.logins[0].fields[0].secret,
            "Protected=\"True\" is a secret"
        );

        assert_eq!(b.otps.len(), 1);
        assert_eq!(b.otps[0].name, "GitHub");
        assert!(b.otps[0].secret.starts_with("otpauth://"));
        assert_eq!(b.logins[1].name, "Bank");
    }

    #[test]
    fn a_file_with_no_entries_is_refused() {
        let empty = br#"<?xml version="1.0"?><KeePassFile><Root><Group><Name>Empty</Name></Group></Root></KeePassFile>"#;
        assert!(parse(empty).is_err());
        assert!(parse(b"not xml").is_err());
    }
}
