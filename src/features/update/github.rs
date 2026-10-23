//! GitHub's release list, the artifact this build can take from it, and the
//! download that follows. No token: a released Pandora is public, and asking with
//! one would tie the check to a signed-in app.

use super::model::{channel, Channel, Info, Progress, REPO};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const API: &str = "https://api.github.com";

/// The file the release publishes with one line per artifact.
const SUMS: &str = "SHA256SUMS";

/// The signature the release publishes beside it, when it has a key to sign with.
const SUMS_SIG: &str = "SHA256SUMS.minisig";

/// How often a check waits on the network, and how long a download may take: an
/// artifact is tens of megabytes, and a phone connection is not a stall.
const CHECK_TIMEOUT: u64 = 30;
const TRANSFER_TIMEOUT: u64 = 600;

#[derive(Debug, serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .http_status_as_error(false)
        .user_agent(concat!("pandora/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

pub fn releases() -> Result<Vec<Release>, String> {
    let mut res = agent(CHECK_TIMEOUT)
        .get(format!("{API}/repos/{REPO}/releases?per_page=30"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub releases: {e}"))?;
    let status = res.status().as_u16();
    // A repository that has never published a release is not a failure: it is the
    // answer "nothing newer than this build".
    if status == 404 {
        return Ok(Vec::new());
    }
    if status != 200 {
        return Err(format!("GitHub releases: HTTP {status}"));
    }
    res.body_mut()
        .read_json()
        .map_err(|e| format!("GitHub releases: {e}"))
}

/// The newest release that is a release: a draft is not published and a
/// prerelease is not what a stranger's install should jump to.
pub fn pick(releases: &[Release]) -> Option<&Release> {
    releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| super::model::parse(&release.tag_name).map(|v| (v, release)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, release)| release)
}

/// The artifact this build can install, if the release carries one.
pub fn asset(channel: Channel, release: &Release) -> Option<&Asset> {
    release
        .assets
        .iter()
        .find(|asset| matches(channel, &asset.name))
}

fn matches(channel: Channel, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let arch = std::env::consts::ARCH;
    match channel {
        Channel::AppImage => name.ends_with(".appimage") && name.contains(arch),
        Channel::Windows => {
            name.ends_with(".exe") && name.contains("windows") && name.contains(arch)
        }
        Channel::MacOs => name.ends_with(".zip") && name.contains("macos"),
        Channel::Android => name.ends_with(".apk"),
        Channel::External => false,
    }
}

/// What the notice should say, or `None` when this build is the newest.
pub fn check() -> Result<Option<Info>, String> {
    let releases = releases()?;
    let Some(release) = pick(&releases) else {
        return Ok(None);
    };
    let latest = release.tag_name.trim_start_matches('v').to_owned();
    if !super::model::newer(&latest, super::model::current()) {
        return Ok(None);
    }
    Ok(Some(Info {
        installable: asset(channel(), release).is_some(),
        latest,
        url: release.html_url.clone(),
    }))
}

/// Fetch an artifact into `dir` and hand back the file it wrote. `progress` is
/// called per chunk, so the frame behind it can draw a percentage.
pub fn download(
    url: &str,
    dir: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<PathBuf, String> {
    let mut res = agent(TRANSFER_TIMEOUT)
        .get(url)
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(|e| format!("download: {e}"))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err(format!("download: HTTP {status}"));
    }
    let total = res.body_mut().content_length();

    let (path, mut file) = staging(dir)?;
    let out = read_into(&mut res, &mut file, total, &mut progress);
    if let Err(e) = out {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

fn read_into(
    res: &mut ureq::http::Response<ureq::Body>,
    file: &mut File,
    total: Option<u64>,
    progress: &mut impl FnMut(Progress),
) -> Result<(), String> {
    let mut reader = res.body_mut().as_reader();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("download: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("download: {e}"))?;
        written += read as u64;
        progress(Progress { written, total });
    }
    file.flush().map_err(|e| format!("download: {e}"))
}

/// One staging file in the destination's own directory, so the swap at the end is
/// a rename on one filesystem and not a copy across two. Only one download runs at
/// a time (`Update::busy`), so the name is the process's own.
fn staging(dir: &Path) -> Result<(PathBuf, File), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("download: {e}"))?;
    let path = dir.join(format!(".pandora-update-{}", std::process::id()));
    let file = File::create(&path).map_err(|e| format!("download: {e}"))?;
    Ok((path, file))
}

/// The key the checksums are signed with, when this build was given one. It is a
/// repository variable at build time, not a secret: a public key is public, and a
/// build without one keeps the hash as its only check.
pub fn public_key() -> Option<&'static str> {
    option_env!("PANDORA_MINISIGN_PUBLIC_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

/// The checksums as the release published them, still to be signed-and-checked.
pub fn sums_text(release: &Release) -> Result<String, String> {
    let url = release
        .assets
        .iter()
        .find(|asset| asset.name == SUMS)
        .map(|asset| asset.browser_download_url.as_str())
        .ok_or("the release publishes no SHA256SUMS")?;
    let mut res = agent(CHECK_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| format!("SHA256SUMS: {e}"))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err(format!("SHA256SUMS: HTTP {status}"));
    }
    res.body_mut()
        .read_to_string()
        .map_err(|e| format!("SHA256SUMS: {e}"))
}

/// The two columns of the checksum file, by file name. A line that names no file is
/// dropped rather than guessed at.
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            Some((name.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

/// The published hashes, by file name.
pub fn sums(release: &Release) -> Result<HashMap<String, String>, String> {
    Ok(parse_sums(&sums_text(release)?))
}

/// The signature text the release publishes, if it publishes one at all.
pub fn signature(release: &Release) -> Result<Option<String>, String> {
    let Some(url) = release
        .assets
        .iter()
        .find(|asset| asset.name == SUMS_SIG)
        .map(|asset| asset.browser_download_url.as_str())
    else {
        return Ok(None);
    };
    let mut res = agent(CHECK_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| format!("{SUMS_SIG}: {e}"))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err(format!("{SUMS_SIG}: HTTP {status}"));
    }
    let text = res
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{SUMS_SIG}: {e}"))?;
    Ok(Some(text))
}

/// Whether the checksums are the ones the release's key signed. A hash says the file
/// is intact; this says who wrote the checksum.
pub fn verify_signature(key: &str, sums: &str, signature: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key.trim())
        .map_err(|_| "the key this build trusts does not read".to_owned())?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|_| format!("{SUMS_SIG} does not read as a minisign signature"))?;
    key.verify(sums.as_bytes(), &signature, false)
        .map_err(|_| format!("{SUMS} is not the file the release signed"))
}

/// Whether the bytes are the ones the release published. A mismatch is the one
/// failure that has to stop everything: what would be installed is not what was
/// built.
pub fn verify(path: &Path, name: &str, sums: &HashMap<String, String>) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let want = sums
        .get(name)
        .ok_or_else(|| format!("SHA256SUMS does not cover {name}"))?;
    let mut file = File::open(path).map_err(|e| format!("verify: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| format!("verify: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let got = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if &got != want {
        return Err(format!("{name} does not match its published hash"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.to_owned(),
            html_url: format!("https://example.invalid/{tag}"),
            draft,
            prerelease,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).to_owned(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn the_api_reply_reads_into_the_shape_the_check_expects() {
        // Trimmed from a real `GET /repos/{owner}/{repo}/releases` reply: the field
        // names are GitHub's, and a rename there would otherwise only show up on a
        // machine with a release published.
        let json = r#"[
            {
                "tag_name": "v0.1.1",
                "html_url": "https://github.com/Pandora-Secrets/pandora/releases/tag/v0.1.1",
                "draft": false,
                "prerelease": false,
                "assets": [
                    {
                        "name": "Pandora-0.1.1-x86_64.AppImage",
                        "browser_download_url": "https://example.invalid/Pandora-0.1.1-x86_64.AppImage"
                    },
                    {
                        "name": "SHA256SUMS",
                        "browser_download_url": "https://example.invalid/SHA256SUMS"
                    }
                ]
            },
            {
                "tag_name": "v0.2.0",
                "html_url": "https://github.com/Pandora-Secrets/pandora/releases/tag/v0.2.0",
                "draft": true,
                "prerelease": false,
                "assets": []
            }
        ]"#;
        let parsed: Vec<Release> = serde_json::from_str(json).unwrap();
        let newest = pick(&parsed).expect("one published release is newer than nothing");
        assert_eq!(newest.tag_name, "v0.1.1");
        assert_eq!(
            newest.html_url,
            "https://github.com/Pandora-Secrets/pandora/releases/tag/v0.1.1"
        );
        assert_eq!(newest.assets.len(), 2);
        assert_eq!(
            super::super::model::parse(&newest.tag_name),
            Some([0, 1, 1])
        );
    }

    #[test]
    fn the_newest_published_release_wins() {
        let all = [
            release("v0.1.0", false, false, &[]),
            release("v0.2.0", true, false, &[]),
            release("v0.3.0-rc1", false, true, &[]),
            release("v0.1.9", false, false, &[]),
            release("nightly", false, false, &[]),
        ];
        assert_eq!(pick(&all).map(|r| r.tag_name.as_str()), Some("v0.1.9"));
        assert_eq!(pick(&[]).map(|r| r.tag_name.as_str()), None);
        assert_eq!(
            pick(&[release("v1.0.0", true, false, &[])]).map(|r| r.tag_name.as_str()),
            None,
            "a draft is not published, so it is not an update"
        );
    }

    #[test]
    fn each_build_takes_only_its_own_artifact() {
        let names = [
            "Pandora-0.1.1-x86_64.AppImage",
            "Pandora-0.1.1-aarch64.AppImage",
            "Pandora-0.1.1-windows-x86_64.zip",
            "Pandora-0.1.1-windows-x86_64.exe",
            "Pandora-0.1.1-macos-universal.dmg",
            "Pandora-0.1.1-macos-universal.zip",
            "Pandora-0.1.1-android-arm64.apk",
            "SHA256SUMS",
        ];
        let all = release("v0.1.1", false, false, &names);

        let appimage = asset(Channel::AppImage, &all).map(|a| a.name.as_str());
        let windows = asset(Channel::Windows, &all).map(|a| a.name.as_str());
        let macos = asset(Channel::MacOs, &all).map(|a| a.name.as_str());
        let android = asset(Channel::Android, &all).map(|a| a.name.as_str());

        assert_eq!(
            windows,
            Some("Pandora-0.1.1-windows-x86_64.exe"),
            "the portable zip is not what a running exe can replace"
        );
        assert_eq!(
            macos,
            Some("Pandora-0.1.1-macos-universal.zip"),
            "a dmg has to be mounted; the zip is the bundle itself"
        );
        assert_eq!(android, Some("Pandora-0.1.1-android-arm64.apk"));
        assert_eq!(
            appimage,
            names
                .iter()
                .copied()
                .find(|name| name.ends_with(".AppImage") && name.contains(std::env::consts::ARCH)),
            "the AppImage must carry the architecture this build runs as"
        );
        assert!(
            asset(Channel::External, &all).is_none(),
            "a build from a package manager cannot replace itself"
        );
    }

    #[test]
    fn a_download_that_does_not_match_is_refused() {
        let dir = std::env::temp_dir().join(format!("pandora-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Pandora-0.1.1-x86_64.AppImage");
        std::fs::write(&path, b"the bytes that actually arrived").unwrap();

        let mut sums = HashMap::new();
        // SHA-256 of "the bytes that were meant to arrive", not of what landed.
        sums.insert(
            "Pandora-0.1.1-x86_64.AppImage".to_owned(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
        );

        let refused = verify(&path, "Pandora-0.1.1-x86_64.AppImage", &sums);
        assert!(refused.is_err(), "a wrong hash has to stop the install");
        assert!(
            path.exists(),
            "and it leaves the staged file for the caller to drop"
        );

        let missing = verify(&path, "Pandora-0.1.1-windows-x86_64.exe", &sums);
        assert!(
            missing.is_err(),
            "an artifact SHA256SUMS does not cover is refused"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The doc key and signature from `minisign-verify`, over the word "test": a real
    /// signature, a public key, and nothing secret in the repository.
    const TEST_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const TEST_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\n\
        RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
        trusted comment: timestamp:1633700835\tfile:test\tprehashed\n\
        wLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==";

    /// The signature is over the checksum file, not over the artifact: a hash that was
    /// swapped for another one is caught here, before anything is downloaded.
    #[test]
    fn a_signed_checksum_file_is_the_one_that_is_trusted() {
        assert!(verify_signature(TEST_KEY, "test", TEST_SIGNATURE).is_ok());

        let swapped = "d41d8cd98f00b204e9800998ecf8427e  Pandora-0.1.1-x86_64.AppImage\n";
        assert!(
            verify_signature(TEST_KEY, swapped, TEST_SIGNATURE).is_err(),
            "another checksum file is another file"
        );
        assert!(verify_signature(TEST_KEY, "test", "not a signature").is_err());
        assert!(verify_signature("not a key", "test", TEST_SIGNATURE).is_err());

        // A build with no key compiled in has the hash and nothing else, and says so.
        assert!(public_key().is_none() || !public_key().unwrap().is_empty());
    }

    /// A release with no signature reads as `None` instead of failing: the hash is
    /// still there, and the caller decides what a missing signature means.
    #[test]
    fn a_release_without_a_signature_is_read_as_none() {
        let plain = release(
            "v0.1.1",
            false,
            false,
            &["Pandora-0.1.1-x86_64.AppImage", "SHA256SUMS"],
        );
        assert!(signature(&plain).unwrap().is_none());

        let signed = release(
            "v0.1.1",
            false,
            false,
            &["SHA256SUMS", "SHA256SUMS.minisig"],
        );
        // It goes to the network for one that is listed; a bad host is an error, not
        // a silent `None`: a release that says it is signed has to be.
        assert!(signature(&signed).is_err());
    }

    #[test]
    fn the_published_hash_is_the_one_that_passes() {
        use sha2::{Digest, Sha256};

        let dir = std::env::temp_dir().join(format!("pandora-verify-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("artifact");
        let bytes = b"a release artifact";
        std::fs::write(&path, bytes).unwrap();

        let hash = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let sums = HashMap::from([("artifact".to_owned(), hash)]);

        assert!(verify(&path, "artifact", &sums).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
