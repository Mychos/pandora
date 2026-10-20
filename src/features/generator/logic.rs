//! Password generation: a recipe of length + character classes, and an
//! unbiased generator. Pure — no egui, shared with the entry editor.

use crate::vault::crypto::random_bytes;

pub const MIN_LEN: usize = 8;
pub const MAX_LEN: usize = 64;

const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.?";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    pub len: usize,
    pub upper: bool,
    pub lower: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            len: 20,
            upper: true,
            lower: true,
            digits: true,
            symbols: true,
        }
    }
}

impl Recipe {
    /// The selected classes, in a fixed order.
    pub fn classes(&self) -> Vec<&'static [u8]> {
        let mut out = Vec::new();
        if self.lower {
            out.push(LOWER);
        }
        if self.upper {
            out.push(UPPER);
        }
        if self.digits {
            out.push(DIGITS);
        }
        if self.symbols {
            out.push(SYMBOLS);
        }
        out
    }
}

/// A password for a caller that only knows how long it wants one: the recipe's
/// default, or the length asked for, clamped to what the recipe allows.
pub fn roll(length: Option<usize>) -> String {
    let mut recipe = Recipe::default();
    if let Some(length) = length {
        recipe.len = length.clamp(MIN_LEN, MAX_LEN);
    }
    generate(&recipe)
}

/// A password of exactly `recipe.len` characters from the selected classes,
/// with at least one from each — empty when no class is selected.
pub fn generate(recipe: &Recipe) -> String {
    let classes = recipe.classes();
    if classes.is_empty() || recipe.len == 0 {
        return String::new();
    }
    let mut alphabet: Vec<u8> = classes.iter().flat_map(|c| c.iter().copied()).collect();
    alphabet.sort_unstable();
    alphabet.dedup();

    let mut out = Vec::with_capacity(recipe.len);
    for class in &classes {
        if out.len() == recipe.len {
            break;
        }
        out.push(class[unbiased(class.len())]);
    }
    while out.len() < recipe.len {
        out.push(alphabet[unbiased(alphabet.len())]);
    }
    shuffle(&mut out);
    String::from_utf8(out).expect("ascii alphabet")
}

/// A random index below `n`, rejecting the tail so every value is equally likely
/// (plain `byte % n` is biased when `n` does not divide 256).
fn unbiased(n: usize) -> usize {
    let limit = 256 - (256 % n);
    loop {
        let b = random_bytes::<1>()[0] as usize;
        if b < limit {
            return b % n;
        }
    }
}

/// Fisher–Yates, unbiased, so the guaranteed class characters land anywhere.
fn shuffle(v: &mut [u8]) {
    for i in (1..v.len()).rev() {
        v.swap(i, unbiased(i + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_honours_length_and_classes() {
        let r = Recipe {
            len: 32,
            upper: true,
            lower: true,
            digits: true,
            symbols: false,
        };
        let pw = generate(&r);
        assert_eq!(pw.chars().count(), 32);
        assert!(pw.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert!(pw.chars().any(|c| c.is_ascii_uppercase()));
        assert!(pw.chars().any(|c| c.is_ascii_lowercase()));
        assert!(pw.chars().any(|c| c.is_ascii_digit()));
        assert!(!pw.chars().any(|c| !c.is_ascii_alphanumeric()));
    }

    #[test]
    fn a_single_class_is_respected_and_an_empty_recipe_yields_nothing() {
        let only = Recipe {
            len: 16,
            upper: false,
            lower: false,
            digits: false,
            symbols: true,
        };
        let pw = generate(&only);
        assert_eq!(pw.chars().count(), 16);
        assert!(pw.bytes().all(|b| SYMBOLS.contains(&b)));

        let none = Recipe {
            len: 16,
            upper: false,
            lower: false,
            digits: false,
            symbols: false,
        };
        assert!(generate(&none).is_empty());
    }

    /// The shuffle must not drop the class guarantee back to chance.
    #[test]
    fn every_requested_class_is_always_present() {
        for _ in 0..200 {
            let r = Recipe {
                len: 8,
                ..Recipe::default()
            };
            let pw = generate(&r);
            assert!(pw.chars().any(|c| c.is_ascii_uppercase()), "{pw}");
            assert!(pw.chars().any(|c| c.is_ascii_lowercase()), "{pw}");
            assert!(pw.chars().any(|c| c.is_ascii_digit()), "{pw}");
            assert!(pw.chars().any(|c| !c.is_ascii_alphanumeric()), "{pw}");
        }
    }
}
