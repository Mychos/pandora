//! Auto-type: the login, typed into whatever window has the focus — for the sites a
//! browser extension cannot fill (a native app, a remote desktop, a terminal with a
//! password prompt).
//!
//! The typing itself is not done with a library but with the small tools the platform
//! already has: `xdotool` on X11, `wtype` (or `ydotool`) on Wayland. They are what a
//! user can install, audit and trust, and a backend that is missing says so instead of
//! silently doing nothing.
//!
//! A sequence is a template: `{USERNAME}`, `{PASSWORD}`, `{OTP}`, `{ENTER}`, `{TAB}`
//! and `{WAIT 250}`. It is expanded here, so what gets typed is testable without a
//! keyboard.

use crate::app::{App, StatusKind};
use crate::shared::i18n::{t, tf};
use crate::vault::model::{now_secs, Login, Vault};
use crate::vault::totp;
use std::process::Command;
use std::time::Duration;
use zeroize::Zeroizing;

/// What a login types when it has no template of its own.
pub const DEFAULT: &str = "{USERNAME}{TAB}{PASSWORD}{ENTER}";

/// How long between the click that asks for it and the first key: long enough to
/// focus the target field, short enough not to feel broken.
pub const LEAD: Duration = Duration::from_millis(2_500);

/// One thing to do, after expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Text(String),
    Key(Key),
    Wait(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Tab,
}

/// The three values a template can ask for.
#[derive(Debug, Default)]
pub struct Values {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub otp: Option<Zeroizing<String>>,
}

/// Expand a template. An unknown `{TOKEN}` is kept as typed rather than dropped: what
/// the user wrote is what they should see going wrong, not a silent hole in a password.
pub fn expand(template: &str, values: &Values) -> Result<Vec<Step>, String> {
    let mut steps: Vec<Step> = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        text.push_str(&rest[..at]);
        let Some(end) = rest[at..].find('}') else {
            // An unclosed brace is text, not a template.
            text.push_str(&rest[at..]);
            rest = "";
            break;
        };
        let token = &rest[at + 1..at + end];
        let upper = token.trim().to_ascii_uppercase();
        let mut matched = true;
        match upper.as_str() {
            "USERNAME" => {
                push(&mut steps, &mut text);
                steps.push(Step::Text(values.username.to_string()));
            }
            "PASSWORD" => {
                push(&mut steps, &mut text);
                steps.push(Step::Text(values.password.to_string()));
            }
            "OTP" => {
                push(&mut steps, &mut text);
                steps.push(Step::Text(
                    values
                        .otp
                        .as_ref()
                        .map(|code| code.to_string())
                        .unwrap_or_default(),
                ));
            }
            "ENTER" => {
                push(&mut steps, &mut text);
                steps.push(Step::Key(Key::Enter));
            }
            "TAB" => {
                push(&mut steps, &mut text);
                steps.push(Step::Key(Key::Tab));
            }
            _ if upper.starts_with("WAIT") => {
                let ms: u32 = token[4..]
                    .trim()
                    .parse()
                    .map_err(|_| format!("{{WAIT}} takes milliseconds, not {token:?}"))?;
                push(&mut steps, &mut text);
                steps.push(Step::Wait(ms.min(60_000)));
            }
            _ => matched = false,
        }
        if !matched {
            text.push('{');
            text.push_str(token);
            text.push('}');
        }
        rest = &rest[at + end + 1..];
    }
    text.push_str(rest);
    push(&mut steps, &mut text);
    Ok(merge(steps))
}

/// Neighbouring text is one step, and nothing empty is a step at all: an `{OTP}` with
/// no seed has to leave the typed password untouched, not split it in two.
fn merge(steps: Vec<Step>) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            Step::Text(text) if text.is_empty() => {}
            Step::Text(text) => match out.last_mut() {
                Some(Step::Text(last)) => last.push_str(&text),
                _ => out.push(Step::Text(text)),
            },
            other => out.push(other),
        }
    }
    out
}

fn push(steps: &mut Vec<Step>, text: &mut String) {
    if !text.is_empty() {
        steps.push(Step::Text(std::mem::take(text)));
    }
}

/// What can type on this machine, if anything.
#[derive(Debug, Clone, Copy)]
pub enum Backend {
    Wtype,
    Xdotool,
    Ydotool,
}

impl Backend {
    pub fn program(&self) -> &'static str {
        match self {
            Backend::Wtype => "wtype",
            Backend::Xdotool => "xdotool",
            Backend::Ydotool => "ydotool",
        }
    }

    pub fn send(&self, step: &Step) -> Result<(), String> {
        match (self, step) {
            (_, Step::Wait(ms)) => {
                std::thread::sleep(Duration::from_millis(u64::from(*ms)));
                Ok(())
            }
            (Backend::Wtype, Step::Text(text)) => self.run(&["-d", "10", text]),
            (Backend::Wtype, Step::Key(Key::Enter)) => self.run(&["-k", "Return"]),
            (Backend::Wtype, Step::Key(Key::Tab)) => self.run(&["-k", "Tab"]),
            (Backend::Xdotool, Step::Text(text)) => {
                self.run(&["type", "--clearmodifiers", "--", text])
            }
            (Backend::Xdotool, Step::Key(Key::Enter)) => {
                self.run(&["key", "--clearmodifiers", "Return"])
            }
            (Backend::Xdotool, Step::Key(Key::Tab)) => {
                self.run(&["key", "--clearmodifiers", "Tab"])
            }
            (Backend::Ydotool, Step::Text(text)) => self.run(&["type", "--", text]),
            (Backend::Ydotool, Step::Key(Key::Enter)) => {
                self.run(&["key", "28:1", "28:0"])
            }
            (Backend::Ydotool, Step::Key(Key::Tab)) => {
                self.run(&["key", "15:1", "15:0"])
            }
        }
    }

    fn run(&self, args: &[&str]) -> Result<(), String> {
        let status = Command::new(self.program())
            .args(args)
            .status()
            .map_err(|e| format!("{}: {e}", self.program()))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{} refused ({status})", self.program()))
        }
    }
}

/// Whether a program is on `PATH`.
pub fn have(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The binary that can type, from the session this process runs in.
pub fn pick() -> Option<Backend> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    if wayland {
        if have("wtype") {
            return Some(Backend::Wtype);
        }
        if have("ydotool") {
            return Some(Backend::Ydotool);
        }
    }
    if x11 && have("xdotool") {
        return Some(Backend::Xdotool);
    }
    None
}

/// The values one login carries: the two fields, and the seed that shares its name.
pub fn values(vault: &Vault, login: &Login) -> Result<Values, String> {
    let otp = vault
        .otps
        .iter()
        .find(|it| it.name == login.name)
        .map(|it| totp::at(&it.secret, now_secs()))
        .transpose()?
        .map(|(code, _left)| code);
    Ok(Values {
        username: Zeroizing::new(login.username.clone()),
        password: login.password.clone(),
        otp,
    })
}

impl App {
    /// Ctrl+T: type the selected login into the window the user is about to focus.
    pub fn autotype(&mut self) {
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        let Some(id) = s.selected.clone() else {
            return self.shell.fail("select the login to type first");
        };
        let Some(login) = s.vault.logins.iter().find(|it| it.id == id).cloned() else {
            return self.shell.fail("select a login to type");
        };
        let values = match values(&s.vault, &login) {
            Ok(values) => values,
            Err(e) => return self.shell.fail(e),
        };
        let template = if login.sequence.trim().is_empty() {
            DEFAULT
        } else {
            login.sequence.as_str()
        };
        let steps = match expand(template, &values) {
            Ok(steps) => steps,
            Err(e) => return self.shell.fail(e),
        };
        let Some(backend) = pick() else {
            return self.shell.fail(t(
                "no way to type on this session — install xdotool (X11) or wtype (Wayland)",
            ));
        };
        self.shell.set_status(
            tf(
                "typing {name} in {seconds} s — click the field now",
                &[("name", &login.name), ("seconds", &LEAD.as_secs())],
            ),
            StatusKind::Info,
        );
        std::thread::spawn(move || {
            std::thread::sleep(LEAD);
            for step in steps {
                if backend.send(&step).is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values {
            username: Zeroizing::new("ana".to_owned()),
            password: Zeroizing::new("hunter2".to_owned()),
            otp: Some(Zeroizing::new("123456".to_owned())),
        }
    }

    /// Every token, in order, with the literal text between them kept.
    #[test]
    fn a_sequence_expands_into_what_is_typed() {
        let steps = expand("{USERNAME}{TAB}{PASSWORD}{ENTER}", &values()).unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Text("ana".to_owned()),
                Step::Key(Key::Tab),
                Step::Text("hunter2".to_owned()),
                Step::Key(Key::Enter),
            ]
        );

        let steps = expand("user: {username} code {OTP}", &values()).unwrap();
        assert_eq!(steps, vec![Step::Text("user: ana code 123456".to_owned())]);
    }

    /// A token nobody defined is kept as written, and a `{WAIT}` that is not a number
    /// is a mistake with a message — never a silently missing pause.
    #[test]
    fn what_is_not_a_token_stays_as_typed() {
        let steps = expand("a{NOPE}b", &values()).unwrap();
        assert_eq!(steps, vec![Step::Text("a{NOPE}b".to_owned())]);

        let steps = expand("x{WAIT 150}y", &values()).unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Text("x".to_owned()),
                Step::Wait(150),
                Step::Text("y".to_owned()),
            ]
        );
        assert!(expand("{WAIT soon}", &values()).is_err());
        assert!(expand("unclosed {USERNAME", &values()).is_ok());
        assert!(expand("", &values()).unwrap().is_empty());
    }

    /// A login with no seed types nothing where the code would go.
    #[test]
    fn a_login_without_a_seed_types_no_code() {
        let mut values = values();
        values.otp = None;
        let steps = expand("x{OTP}y", &values).unwrap();
        assert_eq!(steps, vec![Step::Text("xy".to_owned())]);
    }

    /// The default is the one every manager writes: user, tab, password, enter.
    #[test]
    fn the_default_sequence_is_the_usual_one() {
        assert_eq!(DEFAULT, "{USERNAME}{TAB}{PASSWORD}{ENTER}");
        let steps = expand(DEFAULT, &values()).unwrap();
        assert_eq!(steps.len(), 4);
    }

    /// No tool on `PATH` means no backend: the caller says so instead of typing into
    /// the void.
    #[test]
    fn a_machine_without_the_tools_has_no_backend() {
        assert!(!have("definitely-not-a-program-pandora"));
    }
}
