//! What a review of the box would change: weak and reused passwords, records left
//! alone and seeds this app would not have written. Reads the vault and measures
//! nothing — no request leaves for a report.

use crate::vault::crypto::{self, strength, Strength};
use crate::vault::model::{now_secs, Kind, Login, Otp, Vault};
use crate::vault::totp;
use std::collections::HashMap;

/// A login nobody has touched in this long is worth a look.
pub const STALE_DAYS: u64 = 365;

/// The digits a code may carry; outside this, no form on the internet asks for it.
const DIGITS: std::ops::RangeInclusive<u32> = 6..=8;

/// The period this app writes when a seed does not say.
const PERIOD: u64 = 30;

/// One thing about one record, as a review reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub concern: Concern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Concern {
    /// A password the generator itself would call weak.
    Weak,
    /// The same password in this many records.
    Reused(usize),
    /// Not touched in this many days.
    Stale(u64),
    /// A seed whose period or digit count is not the one this app writes.
    OddCode { digits: u32, period: u64 },
    /// A seed no code comes out of.
    BrokenCode,
    /// The password is in a public breach, this many times. Only a run the user
    /// switched on can find one; `report` never does.
    Breached(u64),
}

impl Concern {
    /// The review's own order: the worst first.
    pub fn rank(&self) -> u8 {
        match self {
            Concern::Breached(_) => 0,
            Concern::Weak => 1,
            Concern::Reused(_) => 2,
            Concern::BrokenCode => 4,
            Concern::OddCode { .. } => 5,
            Concern::Stale(_) => 6,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// One finding per record and concern, worst first.
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    /// Add a finding a caller found on its own (the breach lookup), keeping the
    /// review's order so the screen never has to sort.
    pub fn add(&mut self, finding: Finding) {
        let at = self.findings.partition_point(|it| {
            (it.concern.rank(), it.name.as_str()) <= (finding.concern.rank(), finding.name.as_str())
        });
        self.findings.insert(at, finding);
    }

    /// How many findings a concern counts, for the screen's sections.
    pub fn count(&self, concern: &Concern) -> usize {
        self.findings
            .iter()
            .filter(|it| std::mem::discriminant(&it.concern) == std::mem::discriminant(concern))
            .count()
    }
}

pub fn report(vault: &Vault) -> Report {
    let now = now_secs();
    let mut findings = Vec::new();

    // Reuse is counted on a digest of the password, so two passwords are never
    // compared, and the digest never leaves this function.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for login in &vault.logins {
        if !login.password.is_empty() {
            *seen
                .entry(crypto::digest(login.password.as_bytes()))
                .or_default() += 1;
        }
    }

    for login in &vault.logins {
        let concerns = login_concerns(login, &seen, now);
        findings.extend(concerns.into_iter().map(|concern| Finding {
            id: login.id.clone(),
            name: login.name.clone(),
            kind: Kind::Login,
            concern,
        }));
    }

    for otp in &vault.otps {
        if let Some(concern) = code_concern(otp) {
            findings.push(Finding {
                id: otp.id.clone(),
                name: otp.name.clone(),
                kind: Kind::Otp,
                concern,
            });
        }
    }

    findings.sort_by(|a, b| {
        a.concern
            .rank()
            .cmp(&b.concern.rank())
            .then_with(|| a.name.cmp(&b.name))
    });
    Report { findings }
}

fn login_concerns(login: &Login, seen: &HashMap<String, usize>, now: u64) -> Vec<Concern> {
    let mut concerns = Vec::new();
    if login.password.is_empty() {
        return concerns;
    }
    if strength(&login.password) == Strength::Weak {
        concerns.push(Concern::Weak);
    }
    let count = seen
        .get(&crypto::digest(login.password.as_bytes()))
        .copied()
        .unwrap_or(1);
    if count > 1 {
        concerns.push(Concern::Reused(count));
    }
    let days = now.saturating_sub(login.modified) / 86_400;
    if days >= STALE_DAYS {
        concerns.push(Concern::Stale(days));
    }
    concerns
}

fn code_concern(otp: &Otp) -> Option<Concern> {
    let Some(shape) = totp::shape(&otp.secret) else {
        return Some(Concern::BrokenCode);
    };
    (shape.period != PERIOD || !DIGITS.contains(&shape.digits)).then_some(Concern::OddCode {
        digits: shape.digits,
        period: shape.period,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{Entry, File};
    use zeroize::Zeroizing;

    fn login(name: &str, password: &str, modified: u64) -> Login {
        let mut it = Login::new(modified);
        it.name = name.to_owned();
        it.password = Zeroizing::new(password.to_owned());
        it.modified = modified;
        it
    }

    fn otp(name: &str, secret: &str) -> Otp {
        let mut it = Otp::new(1);
        it.name = name.to_owned();
        it.secret = Zeroizing::new(secret.to_owned());
        it
    }

    #[test]
    fn a_weak_password_and_a_reused_one_are_both_found() {
        let mut v = Vault::new("Box");
        v.logins = vec![
            login("Weak", "hunter2", now_secs()),
            login("Same A", "correct horse battery staple", now_secs()),
            login("Same B", "correct horse battery staple", now_secs()),
        ];
        let r = report(&v);
        assert_eq!(r.count(&Concern::Weak), 1, "only the weak one");
        assert_eq!(
            r.count(&Concern::Reused(2)),
            2,
            "both halves of the shared password"
        );
        let weak = r.findings.iter().find(|it| it.name == "Weak").unwrap();
        assert_eq!(weak.kind, Kind::Login);
        assert_eq!(weak.concern, Concern::Weak);
    }

    #[test]
    fn a_login_left_alone_for_a_year_is_stale() {
        let now = now_secs();
        let mut v = Vault::new("Box");
        v.logins = vec![
            login("Old", "correct horse battery staple", now - 400 * 86_400),
            login("Fresh", "correct horse battery staple", now),
        ];
        let r = report(&v);
        let stale: Vec<&str> = r
            .findings
            .iter()
            .filter(|it| matches!(it.concern, Concern::Stale(_)))
            .map(|it| it.name.as_str())
            .collect();
        assert_eq!(stale, ["Old"]);
        assert_eq!(r.count(&Concern::Reused(2)), 2, "and they are one password");
    }

    #[test]
    fn a_seed_is_judged_by_its_shape_not_by_a_code() {
        let mut v = Vault::new("Box");
        v.otps = vec![
            otp("Plain", "JBSWY3DPEHPK3PXP"),
            otp(
                "Slow",
                "otpauth://totp/ACME?secret=JBSWY3DPEHPK3PXP&period=60",
            ),
            otp(
                "Wide",
                "otpauth://totp/ACME?secret=JBSWY3DPEHPK3PXP&digits=10",
            ),
            otp(
                "Mine",
                "otpauth://totp/ACME?secret=JBSWY3DPEHPK3PXP&period=30&digits=8",
            ),
            otp("Garbage", "not a seed at all"),
        ];
        let r = report(&v);
        let odd: Vec<&str> = r
            .findings
            .iter()
            .filter(|it| matches!(it.concern, Concern::OddCode { .. }))
            .map(|it| it.name.as_str())
            .collect();
        assert_eq!(odd, ["Slow"], "the period, not the hash");
        assert_eq!(
            r.count(&Concern::BrokenCode),
            2,
            "ten digits is a seed no code comes out of"
        );
        assert!(
            r.findings.iter().any(|it| it.name == "Garbage"),
            "a seed no code comes out of is a finding"
        );
    }

    #[test]
    fn a_report_reads_no_note_file_or_address() {
        let mut v = Vault::new("Box");
        let mut note = crate::vault::model::Note::new(1);
        note.name = "Nothing to review".to_owned();
        note.body = Zeroizing::new("hunter2".to_owned());
        v.drive.push(Entry::Note(note));
        v.drive.push(Entry::File(File::new(
            "scan.pdf".to_owned(),
            b"bytes".to_vec(),
            1,
        )));

        assert!(report(&v).is_empty(), "only logins and seeds are reviewed");
    }
}
