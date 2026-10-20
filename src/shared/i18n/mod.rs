use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En = 0,
    Es = 1,
}

impl Lang {
    pub const ALL: [Self; 2] = [Self::En, Self::Es];

    pub fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Es => "Español",
        }
    }

    pub fn tr(self, key: &str) -> &'static str {
        let catalogs = catalogs();
        catalogs[self as usize]
            .get(key)
            .or_else(|| catalogs[Lang::En as usize].get(key))
            .copied()
            .unwrap_or_else(|| {
                static MISSING: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
                let mut missing = MISSING
                    .get_or_init(|| Mutex::new(HashMap::new()))
                    .lock()
                    .expect("translation cache lock");
                *missing
                    .entry(key.to_owned())
                    .or_insert_with(|| Box::leak(key.to_owned().into_boxed_str()))
            })
    }

    pub fn tf(self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        args.iter()
            .fold(self.tr(key).to_owned(), |text, (name, value)| {
                text.replace(&format!("{{{name}}}"), &value.to_string())
            })
    }
}

static LANG: AtomicU8 = AtomicU8::new(Lang::En as u8);

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Es,
        _ => Lang::En,
    }
}

pub fn t(key: &str) -> &'static str {
    lang().tr(key)
}

pub fn text(key: &str) -> Cow<'static, str> {
    let catalogs = catalogs();
    catalogs[lang() as usize]
        .get(key)
        .or_else(|| catalogs[Lang::En as usize].get(key))
        .map_or_else(|| Cow::Owned(key.to_owned()), |text| Cow::Borrowed(*text))
}

pub fn tf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    lang().tf(key, args)
}

fn catalogs() -> &'static [HashMap<&'static str, &'static str>; 2] {
    static CATALOGS: OnceLock<[HashMap<&'static str, &'static str>; 2]> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        [
            parse(include_str!("en.json")),
            parse(include_str!("es.json")),
        ]
    })
}

fn parse(json: &str) -> HashMap<&'static str, &'static str> {
    serde_json::from_str::<HashMap<String, String>>(json)
        .expect("valid translation catalog")
        .into_iter()
        .map(|(key, value)| {
            (
                Box::leak(key.into_boxed_str()) as &'static str,
                Box::leak(value.into_boxed_str()) as &'static str,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_share_keys_and_named_placeholders() {
        let catalogs = catalogs();
        assert_eq!(
            catalogs[Lang::En as usize]
                .keys()
                .collect::<std::collections::HashSet<_>>(),
            catalogs[Lang::Es as usize]
                .keys()
                .collect::<std::collections::HashSet<_>>(),
        );
        assert!(catalogs
            .iter()
            .all(|catalog| catalog.values().all(|value| !value.is_empty())));
        for key in catalogs[Lang::En as usize].keys() {
            assert_eq!(
                placeholders(catalogs[Lang::En as usize][key]),
                placeholders(catalogs[Lang::Es as usize][key]),
                "placeholder mismatch for {key}"
            );
        }
        assert_eq!(Lang::Es.tr("{count} minutes"), "{count} minutos");
        assert_eq!(
            Lang::Es.tf("{count} minutes", &[("count", &3)]),
            "3 minutos"
        );
        assert_eq!(tf("{count} minutes", &[("count", &3)]), "3 minutes");
    }

    fn placeholders(text: &str) -> std::collections::HashSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect()
    }
}
