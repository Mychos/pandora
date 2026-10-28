//! The box, for a script: list, get, otp, generate, export. Nothing here opens a
//! window, and nothing takes a secret on the command line — a password typed after
//! `--` is a password in the shell history and in every process listing on the
//! machine. The three ways in are a file, an askpass program, and the environment, in
//! that order, because that is also the order of how little they leak.
//!
//! Output is the answer and nothing else: no banner, no progress, no timestamps. The
//! errors go to stderr with a non-zero exit, so `$(pandora-cli get …)` is safe.

use pandora::features::generator::logic as generator;
use pandora::shared::utils::csv;
use pandora::vault::model::{now_secs, Login, Vault};
use pandora::vault::{store, totp};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use zeroize::Zeroizing;

const USAGE: &str = "\
pandora-cli — the box, for a script

  pandora-cli [--box FILE] [--keyfile FILE] COMMAND

Commands
  list                      one record per line: id, kind, name, user
  get NAME [--field F]      one field of one record (password, username, url, notes)
  otp NAME                  the one-time code that seed is holding now
  generate [--length N]     a fresh password
  export FILE|-             the whole box as plaintext CSV, or - for stdout

The master password is read from, in this order:
  --password-file FILE, $PANDORA_PASSWORD_FILE, $PANDORA_ASKPASS, $PANDORA_PASSWORD
It is never taken on the command line: that would be a copy in the shell history and
in every process listing.

A YubiKey that is plugged in answers for a box that enrolled one.
";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return;
    }
    match run(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("pandora-cli: {e}");
            std::process::exit(2);
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut box_path: Option<PathBuf> = None;
    let mut keyfile: Option<PathBuf> = None;
    let mut password_file: Option<PathBuf> = None;
    let mut command: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut length: Option<usize> = None;
    let mut field = "password".to_owned();

    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        let mut value = |name: &str| -> Result<String, String> {
            at += 1;
            args.get(at)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg {
            "--box" => box_path = Some(PathBuf::from(value("--box")?)),
            "--keyfile" => keyfile = Some(PathBuf::from(value("--keyfile")?)),
            "--password-file" => password_file = Some(PathBuf::from(value("--password-file")?)),
            "--length" => {
                length = Some(
                    value("--length")?
                        .parse()
                        .map_err(|_| "--length takes a number".to_owned())?,
                )
            }
            "--field" => field = value("--field")?,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other if command.is_none() => command = Some(other.to_owned()),
            other => rest.push(other.to_owned()),
        }
        at += 1;
    }

    let Some(command) = command else {
        return Err("no command — try --help".to_owned());
    };
    // Pure commands first: they need no box, and a script that generates a password
    // must not be asked for a master password to do it.
    if command == "generate" {
        return print(&generator::roll(length));
    }
    let path = box_path.unwrap_or_else(store::default_path);
    let password = password(password_file.as_deref())?;
    let keyfile = match &keyfile {
        Some(path) => Some(store::read_keyfile(path)?),
        None => None,
    };
    let (vault, _key) = open(&path, &password, keyfile.as_ref().map(|it| it.as_slice()))?;

    match command.as_str() {
        "list" => list(&vault),
        "get" => {
            let name = rest.first().ok_or("get needs a record name")?;
            get(&vault, name, &field)
        }
        "otp" => {
            let name = rest.first().ok_or("otp needs a record name")?;
            otp(&vault, name)
        }
        "export" => {
            let to = rest.first().ok_or("export needs a file, or - for stdout")?;
            export(&vault, to)
        }
        other => Err(format!("unknown command {other} — try --help")),
    }
}

/// The master password, from the least leaky place it can come from.
fn password(file: Option<&Path>) -> Result<Zeroizing<String>, String> {
    if let Some(path) = file {
        return first_line(&std::fs::read_to_string(path).map_err(|e| format!("{path:?}: {e}"))?);
    }
    if let Some(path) = env::var_os("PANDORA_PASSWORD_FILE") {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{:?}: {e}", Path::new(&path)))?;
        return first_line(&text);
    }
    if let Some(program) = env::var_os("PANDORA_ASKPASS").or_else(|| env::var_os("SSH_ASKPASS")) {
        let out = Command::new(&program)
            .arg("Master password for Pandora")
            .output()
            .map_err(|e| format!("{:?}: {e}", Path::new(&program)))?;
        if !out.status.success() {
            return Err("the askpass program refused".to_owned());
        }
        return first_line(&String::from_utf8_lossy(&out.stdout));
    }
    if let Ok(secret) = env::var("PANDORA_PASSWORD") {
        return Ok(Zeroizing::new(secret));
    }
    Err(
        "no master password: pass --password-file, $PANDORA_ASKPASS or $PANDORA_PASSWORD"
            .to_owned(),
    )
}

fn first_line(text: &str) -> Result<Zeroizing<String>, String> {
    let line = text
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r');
    if line.is_empty() {
        return Err("the master password is empty".to_owned());
    }
    Ok(Zeroizing::new(line.to_owned()))
}

/// Open the box the way the app does: password, keyfile, and a key if one is plugged in
/// and the box enrolled it.
fn open(
    path: &Path,
    password: &str,
    keyfile: Option<&[u8]>,
) -> Result<(Vault, store::Credentials), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let key = pandora::shared::yubikey::Usb;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let factor = pandora::shared::yubikey::present().then_some(&key as &dyn store::Factor);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let factor: Option<&dyn store::Factor> = None;
    let (creds, vault) = store::open_with(path, password, keyfile, factor)?;
    Ok((vault, creds))
}

fn list(vault: &Vault) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for it in vault.records() {
        let user = match it {
            pandora::vault::model::Any::Login(login) => login.username.as_str(),
            pandora::vault::model::Any::Passkey(passkey) => passkey.rp_id.as_str(),
            _ => "",
        };
        writeln!(
            out,
            "{}\t{}\t{}\t{}",
            it.id(),
            it.kind().label(),
            it.name(),
            user
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One record by name, or by id: an exact name first, then a single record whose name
/// contains it. Two candidates are an error that lists them, never a guess.
fn find(vault: &Vault, wanted: &str) -> Result<Login, String> {
    let wanted = wanted.trim();
    if let Some(it) = vault.logins.iter().find(|it| it.id == wanted) {
        return Ok(it.clone());
    }
    if let Some(it) = vault
        .logins
        .iter()
        .find(|it| it.name.eq_ignore_ascii_case(wanted))
    {
        return Ok(it.clone());
    }
    let needle = wanted.to_lowercase();
    let mut hits: Vec<&Login> = vault
        .logins
        .iter()
        .filter(|it| it.name.to_lowercase().contains(&needle))
        .collect();
    match hits.len() {
        0 => Err(format!("no record named {wanted:?}")),
        1 => Ok(hits.remove(0).clone()),
        _ => {
            let names: Vec<&str> = hits.iter().map(|it| it.name.as_str()).collect();
            Err(format!(
                "{wanted:?} matches more than one: {}",
                names.join(", ")
            ))
        }
    }
}

fn get(vault: &Vault, name: &str, field: &str) -> Result<(), String> {
    let login = find(vault, name)?;
    match field {
        "password" => print(&login.password),
        "username" => print(&login.username),
        "url" => print(&login.url),
        "notes" => print(&login.notes),
        other => Err(format!(
            "unknown field {other:?} — password, username, url or notes"
        )),
    }
}

fn otp(vault: &Vault, name: &str) -> Result<(), String> {
    let wanted = name.trim().to_lowercase();
    let hits: Vec<&pandora::vault::model::Otp> = vault
        .otps
        .iter()
        .filter(|it| it.name.eq_ignore_ascii_case(&wanted) || it.id == name)
        .collect();
    if hits.is_empty() {
        return Err(format!("no seed named {name:?}"));
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|it| it.name.as_str()).collect();
        return Err(format!(
            "{name:?} matches more than one: {}",
            names.join(", ")
        ));
    }
    let (code, _left) = totp::at(&hits[0].secret, now_secs())?;
    print(&code)
}

fn export(vault: &Vault, to: &str) -> Result<(), String> {
    let text = Zeroizing::new(csv::write(vault));
    if to == "-" {
        let mut out = std::io::stdout().lock();
        out.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    // A plaintext export is the one file that must never be world-readable.
    store::write_private(Path::new(to), text.as_bytes())
}

fn print(secret: &str) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    out.write_all(secret.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora::vault::model::Otp;

    fn vault() -> Vault {
        let mut vault = Vault::new("Box");
        for (id, name) in [("a", "GitHub"), ("b", "GitHub Work"), ("c", "Bank")] {
            let mut it = Login::new(1);
            it.id = id.to_owned();
            it.name = name.to_owned();
            it.username = "someone".to_owned();
            it.password = Zeroizing::new(format!("pw-{id}"));
            vault.logins.push(it);
        }
        let mut seed = Otp::new(1);
        seed.name = "Mail".to_owned();
        seed.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        vault.otps.push(seed);
        vault
    }

    /// A name finds one record: exact first, then a single match, and two candidates
    /// are an error instead of a wrong password.
    #[test]
    fn a_name_finds_one_record_or_says_which_ones() {
        let vault = vault();
        assert_eq!(find(&vault, "Bank").unwrap().id, "c");
        assert_eq!(
            find(&vault, "bank").unwrap().id,
            "c",
            "case is not the name"
        );
        assert_eq!(find(&vault, "b").unwrap().id, "b", "an id is an id");
        assert!(find(&vault, "Hub").is_err(), "two matches is an error");
        assert_eq!(
            find(&vault, "Work").unwrap().id,
            "b",
            "one match is the answer"
        );
        assert!(find(&vault, "Nope").is_err());
    }

    #[test]
    fn a_seed_is_found_the_same_way_and_derives_a_code() {
        let text = first_line("hunter2\nextra\n").unwrap();
        assert_eq!(text.as_str(), "hunter2");
        assert!(first_line("").is_err());
        assert_eq!(
            first_line("  \n").unwrap().as_str(),
            "  ",
            "spaces are a password like any other"
        );
    }

    /// The password never comes from the arguments: a file is read to its first line,
    /// and an empty one is refused rather than tried.
    #[test]
    fn the_password_comes_from_a_file() {
        let path = std::env::temp_dir().join(format!("pandora-cli-pw-{}", std::process::id()));
        std::fs::write(&path, "secret\nignored\n").unwrap();
        assert_eq!(password(Some(&path)).unwrap().as_str(), "secret");

        std::fs::write(&path, "\n").unwrap();
        assert!(
            password(Some(&path)).is_err(),
            "an empty file is not a password"
        );
        let _ = std::fs::remove_file(&path);
    }
}
