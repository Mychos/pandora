//! The review pane: one row per finding, ordered by what it costs, each row a jump
//! to the record it names.

use crate::app::App;
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::vault::health::{Concern, Finding, Report};
use crate::vault::model::{Kind, Vault};
use egui::{RichText, Sense, Ui};

const ROW: f32 = 56.0;

pub fn main(ui: &mut Ui, app: &mut App, compact: bool) {
    let Some((key, mut report)) = app
        .boxes
        .session
        .as_ref()
        .map(|s| (box_key(s), crate::vault::health::report(&s.vault)))
    else {
        return;
    };
    if app.prefs.breach_check {
        if !app.health.covers(&key) {
            if let Some(s) = app.boxes.session.as_ref() {
                let vault = &s.vault;
                app.health.start(&key, vault);
            }
        }
        app.health.pump(ui.ctx());
        if let Some(s) = app.boxes.session.as_ref() {
            merge(&mut report, &app.health, &s.vault);
        }
    } else {
        app.health.clear();
    }
    egui::CentralPanel::default()
        .frame(theme::pane(compact))
        .show(ui, |ui| {
            head(ui, &report, app);
            if report.is_empty() {
                theme::empty(
                    ui,
                    false,
                    "nothing to review",
                    "every login, card and code looks fine",
                );
                return;
            }
            let mut jump: Option<(Kind, String)> = None;
            let icons = &app.icons;
            theme::scroll(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for finding in &report.findings {
                        if finding_row(ui, icons, finding).clicked() {
                            jump = Some((finding.kind, finding.id.clone()));
                        }
                    }
                },
            );
            if let Some((kind, id)) = jump {
                app.boxes.reveal(kind, &id);
            }
        });
}

/// The box a set of answers belongs to: a digest, so nothing here spells a path.
fn box_key(s: &crate::app::Session) -> String {
    crate::vault::crypto::digest(s.path.to_string_lossy().as_bytes())
}

/// What the lookup found, folded into the same list as everything else.
fn merge(report: &mut Report, check: &crate::features::health::Check, vault: &Vault) {
    for (id, count) in &check.found {
        if let Some(it) = vault.any(id) {
            report.add(Finding {
                id: id.clone(),
                name: it.name().to_owned(),
                kind: it.kind(),
                concern: Concern::Breached(*count),
            });
        }
    }
}

fn head(ui: &mut Ui, report: &Report, app: &App) {
    ui.label(
        RichText::new(t("Review"))
            .strong()
            .color(theme::palette().text),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new(match report.findings.len() {
            1 => t("1 thing to look at").to_owned(),
            count => tf("things to look at: {count}", &[("count", &count)]),
        })
        .small()
        .color(theme::palette().muted),
    );
    if app.prefs.breach_check {
        ui.add_space(2.0);
        ui.label(
            RichText::new(status(&app.health))
                .small()
                .color(theme::palette().faint),
        );
    }
    ui.add_space(12.0);
}

/// What the optional lookup is doing, in its own words.
fn status(check: &crate::features::health::Check) -> String {
    if check.running() {
        return t("checking passwords…").to_owned();
    }
    if let Some(e) = &check.error {
        return tf("could not check passwords: {error}", &[("error", e)]);
    }
    tf(
        "{checked} passwords checked, {found} in breaches",
        &[("checked", &check.asked), ("found", &check.found.len())],
    )
}

/// The switch the review's one lookup hangs on: off by default, and the row says
/// what leaves the machine when it is on.
pub fn row(ui: &mut Ui, app: &mut App) {
    let mut on = app.prefs.breach_check;
    if theme::check(ui, &mut on, t("check passwords against public breaches")).changed() {
        app.prefs.breach_check = on;
        if !on {
            app.health.clear();
        }
        crate::features::settings::model::save(&app.prefs);
    }
    if on {
        ui.add_space(2.0);
        theme::hint(ui, &status(&app.health));
        theme::hint(
            ui,
            t("only a five-character hash prefix is sent, never a password or an address"),
        );
    }
}

/// One finding: what the record is, its name, and what a review would change.
fn finding_row(ui: &mut Ui, icons: &theme::Icons, finding: &Finding) -> egui::Response {
    let concern = describe(&finding.concern);
    let label = format!("{}, {concern}", finding.name);
    let (resp, _, ()) = record::row(
        ui,
        ui.id().with(("finding", &finding.id)),
        ROW,
        Sense::click(),
        false,
        &label,
        |ui| {
            record::glyph(ui, icons, finding.kind, None, 15.0);
            let name_h = ui.text_style_height(&egui::TextStyle::Body);
            let concern_h = ui.text_style_height(&egui::TextStyle::Small);
            ui.allocate_ui_with_layout(
                egui::vec2(
                    ui.available_width(),
                    name_h + ui.spacing().item_spacing.y + concern_h,
                ),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&finding.name)
                                .strong()
                                .color(theme::palette().text),
                        )
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&concern)
                                .small()
                                .color(warning(&finding.concern)),
                        )
                        .truncate(),
                    );
                },
            );
            (None, ())
        },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The colour a finding wears: the ones that need a new password first.
fn warning(concern: &Concern) -> egui::Color32 {
    match concern {
        Concern::Breached(_) | Concern::Weak | Concern::Reused(_) | Concern::BrokenCode => {
            theme::palette().error
        }
        _ => theme::palette().muted,
    }
}

/// One line for a finding, in the reader's language.
pub fn describe(concern: &Concern) -> String {
    match concern {
        Concern::Weak => t("weak password").to_owned(),
        Concern::Reused(count) => tf("the same password in {count} records", &[("count", count)]),
        Concern::Stale(days) => tf("not changed in {days} days", &[("days", days)]),
        Concern::Expired(when) => tf("expired in {when}", &[("when", when)]),
        Concern::OddCode { digits, period } => tf(
            "code of {digits} digits every {period} s",
            &[("digits", digits), ("period", period)],
        ),
        Concern::BrokenCode => t("this seed gives no code").to_owned(),
        Concern::Breached(count) => tf(
            "found in public breaches {count} times",
            &[("count", count)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Screen, Tab};
    use crate::shared::ui::shot;
    use crate::vault::model::{Card, Login, Vault};
    use zeroize::Zeroizing;

    #[test]
    fn a_finding_says_what_a_review_would_change() {
        assert_eq!(describe(&Concern::Weak), "weak password");
        assert_eq!(
            describe(&Concern::Reused(3)),
            "the same password in 3 records"
        );
        assert_eq!(describe(&Concern::Stale(400)), "not changed in 400 days");
        assert_eq!(
            describe(&Concern::Expired("01/20".to_owned())),
            "expired in 01/20"
        );
        assert_eq!(
            describe(&Concern::OddCode {
                digits: 6,
                period: 60
            }),
            "code of 6 digits every 60 s"
        );
        assert_eq!(describe(&Concern::BrokenCode), "this seed gives no code");
        assert_eq!(
            describe(&Concern::Breached(42)),
            "found in public breaches 42 times"
        );
    }

    fn painted() -> Vec<String> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.open("t");

        let mut login = Login::new(1);
        login.name = "Bank".to_owned();
        login.password = Zeroizing::new("hunter2".to_owned());
        let mut card = Card::new(1);
        card.name = "Visa".to_owned();
        card.expiry = "01/20".to_owned();

        if let Some(s) = app.boxes.session.as_mut() {
            s.vault.logins.push(login);
            s.vault.cards.push(card);
            s.tab = Tab::Health;
        }

        let shapes = shot::frame(&ctx, egui::vec2(900.0, 600.0), Vec::new(), |ui| {
            main(ui, &mut app, false);
        });
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_pane_lists_what_the_review_found() {
        let said = painted();
        for line in [
            "Review",
            "Bank",
            "weak password",
            "Visa",
            "expired in 01/20",
        ] {
            assert!(
                said.iter().any(|word| word == line),
                "{line:?} is missing: {said:?}"
            );
        }
    }

    /// The lookup's own findings join the same list, at the top, without the screen
    /// having to know where they came from.
    #[test]
    fn a_breached_password_joins_the_review_first() {
        let mut vault = Vault::new("Box");
        let mut login = Login::new(1);
        login.id = "bank".to_owned();
        login.name = "Bank".to_owned();
        login.password = Zeroizing::new("correct horse battery staple".to_owned());
        login.modified = crate::vault::model::now_secs();
        vault.logins.push(login);

        let mut report = crate::vault::health::report(&vault);
        let mut check = crate::features::health::Check::default();
        check.found = vec![("bank".to_owned(), 42)];
        merge(&mut report, &check, &vault);

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].kind, Kind::Login);
        assert_eq!(report.findings[0].concern, Concern::Breached(42));
        assert!(!report.is_empty(), "and the pane has a row to draw");
    }

    #[test]
    fn an_empty_report_is_not_a_list_of_nothing() {
        let mut vault = Vault::new("Box");
        vault.logins.push({
            let mut it = Login::new(1);
            it.password = Zeroizing::new("correct horse battery staple".to_owned());
            it.modified = crate::vault::model::now_secs();
            it
        });
        assert!(crate::vault::health::report(&vault).is_empty());
    }
}
