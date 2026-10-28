//! The two catalogs and the one lookup that reads them. A key is the English
//! sentence itself, so a missing translation is visible in the source.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En = 0,
    Es = 1,
    Fr = 2,
}

impl Lang {
    pub const ALL: [Self; 3] = [Self::En, Self::Es, Self::Fr];

    pub fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Es => "Español",
            Self::Fr => "Français",
        }
    }

    /// The catalog's own words for a key in this language, the English ones when it
    /// does not carry it, and the key itself when no catalog does.
    pub fn tr(self, key: &str) -> &str {
        entry(self, key).unwrap_or(key)
    }

    pub fn tf(self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        args.iter()
            .fold(self.tr(key).to_owned(), |text, (name, value)| {
                text.replace(&format!("{{{name}}}"), &value.to_string())
            })
    }
}

/// One key in one language: the only lookup, so the fallback order lives in one
/// place. `None` when no catalog carries the key at all.
fn entry(lang: Lang, key: &str) -> Option<&'static str> {
    let catalogs = catalogs();
    catalogs[lang as usize]
        .get(key)
        .or_else(|| catalogs[Lang::En as usize].get(key))
        .copied()
}

static LANG: AtomicU8 = AtomicU8::new(Lang::En as u8);

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Es,
        2 => Lang::Fr,
        _ => Lang::En,
    }
}

/// The text for a key in the language on screen, borrowed from the catalog — or the
/// key itself, which is a sentence, when nothing carries it.
pub fn t(key: &str) -> &str {
    lang().tr(key)
}

/// The same text for a caller that has to keep it.
pub fn text(key: &str) -> Cow<'_, str> {
    Cow::Borrowed(t(key))
}

pub fn tf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    lang().tf(key, args)
}

fn catalogs() -> &'static [HashMap<&'static str, &'static str>; 3] {
    static CATALOGS: OnceLock<[HashMap<&'static str, &'static str>; 3]> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        [
            parse(include_str!("en.json")),
            parse(include_str!("es.json")),
            parse(include_str!("fr.json")),
        ]
    })
}

/// Both catalogs are `include_str!`d, so every key and value is a slice of the
/// binary: nothing is copied and nothing leaks.
fn parse(json: &'static str) -> HashMap<&'static str, &'static str> {
    serde_json::from_str(json).expect("valid translation catalog")
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
        assert_eq!(
            catalogs[Lang::En as usize]
                .keys()
                .collect::<std::collections::HashSet<_>>(),
            catalogs[Lang::Fr as usize]
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
            assert_eq!(
                placeholders(catalogs[Lang::En as usize][key]),
                placeholders(catalogs[Lang::Fr as usize][key]),
                "placeholder mismatch for {key} (fr)"
            );
        }
        assert_eq!(Lang::Es.tr("{count} minutes"), "{count} minutos");
        assert_eq!(
            Lang::Es.tf("{count} minutes", &[("count", &3)]),
            "3 minutos"
        );
        assert_eq!(tf("{count} minutes", &[("count", &3)]), "3 minutes");
    }

    /// A key no catalog carries comes back as itself: a sentence on screen, and
    /// nothing allocated or leaked to say so.
    #[test]
    fn a_key_with_no_translation_reads_as_itself() {
        assert_eq!(
            t("a sentence nobody translated"),
            "a sentence nobody translated"
        );
        assert_eq!(
            Lang::Es.tr("a sentence nobody translated"),
            "a sentence nobody translated"
        );
        assert_eq!(entry(Lang::En, "a sentence nobody translated"), None);
    }

    /// Every word the auto-type pane paints is in both hand-written catalogs: the
    /// lookup falls back to English, so a key missing from `es` reads as English on
    /// a Spanish screen and no other test would say so.
    #[test]
    fn the_auto_type_pane_is_translated() {
        let [en, es, fr] = catalogs();
        for key in [
            "Auto-type",
            "Current",
            "Fields",
            "Custom",
            "Types into another window",
            "Only when the focused window is",
            "window title, optional",
            "the login's username",
            "the login's password",
            "the one-time code",
            "waits 250 ms",
            "{key} types it into the focused window",
            "you have {seconds} seconds to click the field",
            "not checked in this session — this desktop cannot report the window title",
            "this session cannot type for you — install xdotool (X11) or wtype (Wayland)",
            "no way to type on this session — install xdotool (X11) or wtype (Wayland)",
            "typing into {title} — it does not match {wanted}",
            "choose icon",
            "no site will match this address",
        ] {
            assert!(en.contains_key(key), "en has no {key}");
            assert!(es.contains_key(key), "es has no {key}");
            assert!(fr.contains_key(key), "fr has no {key}");
            assert_ne!(en[key], es[key], "es still says {key}");
            assert_ne!(en[key], fr[key], "fr still says {key}");
        }
    }

    fn placeholders(text: &str) -> std::collections::HashSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect()
    }
}
