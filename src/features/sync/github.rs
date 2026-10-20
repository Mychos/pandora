//! GitHub as a plain encrypted-blob mirror: the device flow mints a token and
//! the Contents API moves one file. No git — Android and iOS ship no git binary,
//! and a single opaque box has nothing to merge anyway.
//!
//! The session renews itself: `offline_access` asks for an access token with a
//! life, and for the refresh token that trades for the next one. The token is
//! spent before it runs out rather than after. An app that issues neither is
//! used exactly as before — one token, and it does not run out.
//!
//! Every call here blocks, so it runs on a worker thread and lands in
//! `App::pump_sync`; the UI thread never waits on the network.

use crate::shared::i18n::{t, tf};
use crate::vault::codec;
use crate::vault::model::Vault;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// The OAuth App this build signs in as. Public on purpose: the device flow
/// sends no client secret, so this is not a secret either. Register an app with
/// device flow enabled ("Enable Device Flow") and put its client id in
/// `PANDORA_GITHUB_CLIENT_ID` (`.cargo/config.toml`, out of the repo).
const CLIENT_ID: &str = match option_env!("PANDORA_GITHUB_CLIENT_ID") {
    Some(id) => id,
    None => "REPLACE_WITH_YOUR_OAUTH_APP_CLIENT_ID",
};

/// Enough to read and write the private repo holding the boxes, and to resolve
/// the organisations the account belongs to. Without `read:org` GitHub knows of
/// no organisation, and `/user/repos` answers with the personal repos alone —
/// however `affiliation` is spelled.
///
/// `offline_access` asks for an expiring session and the refresh token that
/// renews it, whatever the app's own setting says. It is not a permission and
/// GitHub files it nowhere the user sees; an instance that does not know it
/// answers a session with no refresh token and no expiry, and everything here
/// works without one.
const SCOPE: &str = "repo read:org offline_access";

/// What GitHub puts in front of a refresh token. The remembered half of a
/// session is told apart by it, so the credential store needs no format of its
/// own.
pub const REFRESH_PREFIX: &str = "ghr_";

/// How long before it runs out an access token is traded in. Nothing goes out
/// carrying a token that would die mid-flight.
pub const RENEW_WINDOW: Duration = Duration::from_secs(300);

/// How long a renewal that never reached GitHub waits before the next try. A
/// transport error is not an answer about the session, so nothing may be decided
/// from it — and without a wait a frame loop becomes a request storm.
pub const RENEW_RETRY: Duration = Duration::from_secs(60);

/// What GitHub documents as the life of an access token, used only for one that
/// says it can be renewed and forgets to say when.
const ACCESS_TTL: u64 = 8 * 3600;

const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const OAUTH: &str = "https://github.com";
const API: &str = "https://api.github.com";

/// The largest file this will send or take. GitHub caps a blob at 100 MB, which is
/// the only ceiling that matters here: the Contents API stops returning `content`
/// past 1 MB, so a bigger box is read through the Git Blobs API instead of being
/// refused. A box stays small in practice for another reason — each drive file is
/// capped at 8 MiB.
pub const MAX_FILE: usize = 100 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        // `retry-after` on a refusal lives on the response ureq would otherwise
        // throw away, so the status is read by hand.
        .http_status_as_error(false)
        .user_agent(concat!("pandora/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn net(e: impl std::fmt::Display) -> String {
    tf("GitHub could not be reached — {error}", &[("error", &e)])
}

/// Seconds GitHub asked to be left alone for: `retry-after` when it is sent,
/// otherwise `x-ratelimit-reset` in epoch seconds, otherwise a minute.
fn quiet_for(headers: &ureq::http::HeaderMap) -> u64 {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    if let Some(secs) = header("retry-after") {
        return secs;
    }
    let reset = header("x-ratelimit-reset").unwrap_or(0);
    reset
        .saturating_sub(crate::vault::model::now_secs())
        .max(60)
}

/// Whether a refusal is the hourly budget and not a permission. A secondary
/// limit sends `retry-after`; the primary one is spent down to zero before it
/// starts refusing. A 403 without either is GitHub saying no.
fn throttled(headers: &ureq::http::HeaderMap) -> bool {
    headers.get("retry-after").is_some()
        || headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0")
}

/// What GitHub answers a token it will not take. No retry fixes it, so the
/// caller drops the session instead of asking again. Kept in English: it is the
/// sentinel [`expired`] matches, and it is translated where it is drawn.
pub const EXPIRED: &str = "GitHub: your session is no longer valid — sign in again";

/// Whether a refusal is that: the session itself is gone, and not the request.
pub fn expired(why: &str) -> bool {
    why == EXPIRED
}

/// What a renewal GitHub refused is worded with. The two ways a renewal fails are
/// not the same wall: a refusal is the session being gone, a transport error says
/// nothing about it, and only the first may drop what is in hand. English, like
/// [`EXPIRED`]: [`dead`] matches on it, and the catalog translates it when drawn.
pub const DEAD: &str = "GitHub: the session could not be renewed — sign in again";

/// Whether a failed renewal is that refusal, and not the network.
pub fn dead(why: &str) -> bool {
    why.starts_with(DEAD)
}

/// The token another instance left behind while ours was being spent. GitHub
/// rotates the refresh token on every trade, so a store that no longer holds the
/// one we just used is not a dead session — it is someone else's live one.
pub fn takeover(spent: &str, kept: Option<String>) -> Option<String> {
    kept.filter(|token| token != spent)
}

/// A signed-in session: the access token requests carry, the refresh token that
/// trades for the next one when GitHub sends one, and when the access token runs
/// out. No refresh token means the app is not issuing expiring sessions — the
/// access token is then the session, and it does not run out.
pub struct Session {
    pub access: Zeroizing<String>,
    pub refresh: Option<Zeroizing<String>>,
    pub expires_at: Option<Instant>,
}

/// What both token endpoints answer: a session, or why there is none.
#[derive(serde::Deserialize)]
struct Token {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

impl Token {
    /// The session this answer minted. Only called when it carries a token.
    fn into_session(mut self) -> Session {
        let expires_in = self
            .expires_in
            .or_else(|| self.refresh_token.as_ref().map(|_| ACCESS_TTL));
        Session {
            access: Zeroizing::new(self.access_token.take().unwrap_or_default()),
            refresh: self.refresh_token.map(Zeroizing::new),
            expires_at: expires_in.map(|secs| Instant::now() + Duration::from_secs(secs)),
        }
    }
}

/// A refusal worth explaining: the raw status tells the user nothing. None for
/// a status that went through.
fn denied(status: u16, headers: &ureq::http::HeaderMap) -> Option<String> {
    match status {
        200..=299 => None,
        401 => Some(EXPIRED.to_owned()),
        403 if throttled(headers) => Some(rate_limited(headers)),
        // The organisation enabled OAuth App access restrictions and has not
        // approved this app: no scope this side can ask for gets past it.
        403 => Some(t("GitHub refused: an organisation may be blocking this app").to_owned()),
        429 => Some(rate_limited(headers)),
        404 => Some(
            t("GitHub has no such repo or file (a private one needs the repo scope)").to_owned(),
        ),
        // A create that found an existing file: the mirror has no sha to replace,
        // so it has to come in before it can go out.
        422 => Some(t("that file is already in the repo — get its copy first").to_owned()),
        c => Some(tf("GitHub refused with HTTP {status}", &[("status", &c)])),
    }
}

/// How long GitHub asked to be left alone, in a sentence: the one refusal that
/// carries a number, so it is built here and not looked up.
fn rate_limited(headers: &ureq::http::HeaderMap) -> String {
    tf(
        "GitHub is rate-limiting — wait {minutes} min",
        &[("minutes", &quiet_for(headers).div_ceil(60))],
    )
}

fn configured() -> Result<(), String> {
    if CLIENT_ID.starts_with("REPLACE_") {
        return Err(
            t("this build cannot sign in to GitHub — it was built without an app id").to_owned(),
        );
    }
    Ok(())
}

/// The code the user types into a browser, and the handle the app polls with.
pub struct Device {
    code: String,
    handle: String,
    /// Where the user types it; shown next to the code.
    pub verify: String,
    /// What the browser opens: GitHub's own page with the code already in it when
    /// it sends one, the code appended otherwise — so nothing is ever typed. The
    /// code is still drawn on screen, because a browser is not guaranteed.
    pub open: String,
    interval: Duration,
}

impl Device {
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// Where the user signs in: `page` with the code already in it, so GitHub's page
/// comes up with the code typed and signing in is one tap. GitHub reads the code
/// off `user_code`, which is the contract; the login card's own link opens the
/// same page the browser was handed.
pub fn device_url(page: &str, code: &str) -> String {
    format!("{page}?user_code={code}")
}

/// The page the browser opens: GitHub's own `verification_uri_complete` when it
/// sends one, `device_url` otherwise. Either way the code is already there.
fn open_uri(complete: Option<String>, verify: &str, code: &str) -> String {
    complete
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| device_url(verify, code))
}

/// Ask GitHub for a user code. The user then types it at `Device::verify`.
pub fn start() -> Result<Device, String> {
    #[derive(serde::Deserialize)]
    struct Reply {
        device_code: String,
        user_code: String,
        verification_uri: String,
        // Undocumented in the device flow's own table, but GitHub sends it — it
        // is RFC 8628's own field, and what every other client opens.
        #[serde(default)]
        verification_uri_complete: Option<String>,
        #[serde(default = "interval")]
        interval: u64,
    }
    fn interval() -> u64 {
        5
    }

    configured()?;
    let auth = agent();
    let reply = auth
        .post(format!("{OAUTH}/login/device/code"))
        .header("Accept", "application/json")
        .send_form([("client_id", CLIENT_ID), ("scope", SCOPE)])
        .map_err(net)?
        .body_mut()
        .read_json::<Reply>()
        .map_err(net)?;

    Ok(Device {
        open: open_uri(
            reply.verification_uri_complete,
            &reply.verification_uri,
            &reply.user_code,
        ),
        code: reply.user_code,
        handle: reply.device_code,
        verify: reply.verification_uri,
        // GitHub asks for this many seconds between polls and no fewer.
        interval: Duration::from_secs(reply.interval.max(5)),
    })
}

/// Block until the user authorises in the browser, then hand back the session.
/// `authorization_pending` is the normal answer until they do; `slow_down` asks
/// for a longer wait; anything else ends the flow.
pub fn await_token(mut device: Device) -> Result<Session, String> {
    loop {
        thread::sleep(device.interval);
        let auth = agent();
        let mut res = auth
            .post(format!("{OAUTH}/login/oauth/access_token"))
            .header("Accept", "application/json")
            .send_form([
                ("client_id", CLIENT_ID),
                ("device_code", device.handle.as_str()),
                ("grant_type", GRANT),
            ])
            .map_err(net)?;
        // This body is where the token first lands, and a buffer that is only
        // freed keeps it: the reply is read into something that wipes itself.
        let body = Zeroizing::new(res.body_mut().read_to_string().map_err(net)?);
        let reply: Token = serde_json::from_str(&body).map_err(net)?;

        if reply.access_token.is_some() {
            return Ok(reply.into_session());
        }
        match reply.error.as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => device.interval += Duration::from_secs(5),
            Some("access_denied") => return Err(t("sign-in cancelled").to_owned()),
            Some("expired_token") => return Err(t("the code expired — sign in again").to_owned()),
            Some(_) => return Err(t("GitHub refused the sign-in — try again").to_owned()),
            None => return Err(t("GitHub sent no answer to the sign-in — try again").to_owned()),
        }
    }
}

/// Trade a refresh token for the next session. GitHub takes the refresh token
/// out of use the moment this succeeds, and the access token with it, so what
/// comes back replaces the session and nothing keeps the old one.
pub fn refresh(token: &str) -> Result<Session, String> {
    let body = Zeroizing::new(
        agent()
            .post(format!("{OAUTH}/login/oauth/access_token"))
            .header("Accept", "application/json")
            .send_form([
                ("client_id", CLIENT_ID),
                // The token was minted by the device flow, which sends no
                // secret, so it is renewed with no secret either.
                ("grant_type", "refresh_token"),
                ("refresh_token", token),
            ])
            .map_err(net)?
            .body_mut()
            .read_to_string()
            .map_err(net)?,
    );
    let reply: Token = serde_json::from_str(&body).map_err(net)?;
    if reply.access_token.is_some() {
        return Ok(reply.into_session());
    }
    Err(DEAD.to_owned())
}

pub struct Blob {
    pub bytes: Vec<u8>,
    /// The blob sha. The next push names it to prove it is replacing the version
    /// it read, which is what turns a concurrent edit into a conflict and not a
    /// silently lost box.
    pub sha: String,
}

#[derive(serde::Deserialize)]
struct Content {
    sha: String,
    #[serde(default)]
    content: String,
}

/// What a write answers: the file it wrote, under `content`. A read spells that
/// same key for the base64 of the bytes; a write spells it for the file object,
/// so the read's type cannot hold this one. The `sha` is the file's own blob,
/// which is what the next write names to prove it is replacing this version.
#[derive(serde::Deserialize)]
struct Saved {
    content: SavedFile,
}

#[derive(serde::Deserialize)]
struct SavedFile {
    sha: String,
}

/// The API wraps the base64 every 60 characters, and the STANDARD engine refuses
/// whitespace.
fn payload(wrapped: &str) -> Result<Vec<u8>, String> {
    let flat: String = wrapped.chars().filter(|c| !c.is_whitespace()).collect();
    codec::decode(&flat)
}

pub fn get(repo: &str, file: &str, token: &str) -> Result<Blob, String> {
    get_at(API, repo, file, token)
}

/// [`get`] against one API base, so the read can be tested against a server that is
/// not GitHub's — the same door `write` and `erase_at` go through.
pub fn get_at(base: &str, repo: &str, file: &str, token: &str) -> Result<Blob, String> {
    let header = Zeroizing::new(format!("Bearer {token}"));
    let call = agent();
    let mut res = call
        .get(format!("{base}/repos/{repo}/contents/{file}"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(net)?;
    if let Some(why) = denied(res.status().as_u16(), res.headers()) {
        return Err(why);
    }
    let content: Content = res.body_mut().read_json().map_err(net)?;

    // Past 1 MB the Contents API answers with the file's `sha` and no `content`:
    // the metadata is there, the bytes are not. The Git Blobs API takes that same
    // sha and hands back the bytes (up to 100 MB), so the read falls back to it
    // rather than refusing a box that is perfectly valid.
    let bytes = if content.content.trim().is_empty() {
        blob_at(base, repo, &content.sha, token)?
    } else {
        payload(&content.content)?
    };
    if bytes.is_empty() || bytes.len() > MAX_FILE {
        return Err(t("that repo file is not a box this can open").to_owned());
    }
    Ok(Blob {
        bytes,
        sha: content.sha,
    })
}

/// A blob's bytes by sha, for the file the Contents API would not return. The API
/// documents base64 up to 100 MB; past that it answers `encoding: "none"` and no
/// content at all, which is not a box either way.
pub fn blob_at(base: &str, repo: &str, sha: &str, token: &str) -> Result<Vec<u8>, String> {
    #[derive(serde::Deserialize)]
    struct BlobReply {
        #[serde(default)]
        content: String,
        #[serde(default)]
        encoding: String,
    }

    let header = Zeroizing::new(format!("Bearer {token}"));
    let mut res = agent()
        .get(format!("{base}/repos/{repo}/git/blobs/{sha}"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(net)?;
    if let Some(why) = denied(res.status().as_u16(), res.headers()) {
        return Err(why);
    }
    let reply: BlobReply = res.body_mut().read_json().map_err(net)?;
    if reply.encoding != "base64" {
        return Err(t("that repo file is too large to read as a box").to_owned());
    }
    payload(&reply.content)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// `owner/name`, the only shape the Contents API takes.
    pub full_name: String,
    /// The account or organisation that owns it: what the picker groups the
    /// repos by, and the only thing it needs from the API to draw an avatar.
    pub owner: String,
    pub private: bool,
}

/// Each owner's avatar bytes, by login. Only what the picker draws.
pub type Avatars = HashMap<String, Vec<u8>>;

/// Repos asked for per page, and the most GitHub will answer with: a full page is
/// the signal that there is another one.
const PAGE: usize = 100;

/// The side of the avatar asked for. Big enough for a chip on a HiDPI screen,
/// small enough that the whole reply is a couple of KB per owner.
const AVATAR: u32 = 64;

/// An owner's avatar, straight off the web host. Not the API: it costs no
/// request of the hourly budget and wants no token. `None` for anything that did
/// not come back a small image — a missing avatar must never cost a repo.
fn avatar(call: &ureq::Agent, owner: &str) -> Option<Vec<u8>> {
    let mut res = call
        .get(format!("https://github.com/{owner}.png?size={AVATAR}"))
        .call()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let bytes = res.body_mut().read_to_vec().ok()?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 {
        return None;
    }
    Some(bytes)
}

/// The repos the token can reach, most recently pushed first, every page of
/// them, with each owner's avatar. Private ones are included: the device flow
/// already asks for the `repo` scope. `affiliation` is left at GitHub's default
/// (`owner,collaborator,organization_member`), the only value that covers all
/// three — but it reaches an organisation's repos only while the token carries
/// `read:org` and the organisation has not blocked the app.
pub fn repos(token: &str) -> Result<(Vec<Repo>, Avatars), String> {
    #[derive(serde::Deserialize)]
    struct Raw {
        full_name: String,
        #[serde(default)]
        private: bool,
    }

    let header = Zeroizing::new(format!("Bearer {token}"));
    let call = agent();
    let mut raw: Vec<Raw> = Vec::new();
    for page in 1.. {
        let mut res = call
            .get(format!(
                "{API}/user/repos?per_page={PAGE}&page={page}&sort=updated"
            ))
            .header("Authorization", header.as_str())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(net)?;
        if let Some(why) = denied(res.status().as_u16(), res.headers()) {
            return Err(why);
        }
        let chunk: Vec<Raw> = res.body_mut().read_json().map_err(net)?;
        let last = chunk.len() < PAGE;
        raw.extend(chunk);
        if last {
            break;
        }
    }

    let owned: Vec<Repo> = raw
        .into_iter()
        .map(|r| Repo {
            owner: r
                .full_name
                .split('/')
                .next()
                .unwrap_or(&r.full_name)
                .to_owned(),
            full_name: r.full_name,
            private: r.private,
        })
        .collect();

    let mut owners: Vec<&str> = owned.iter().map(|r| r.owner.as_str()).collect();
    owners.sort_unstable();
    owners.dedup();
    let avatars = owners
        .into_iter()
        .filter_map(|owner| avatar(&call, owner).map(|bytes| (owner.to_owned(), bytes)))
        .collect();
    Ok((owned, avatars))
}

#[derive(serde::Deserialize)]
struct Entry {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

/// The box file names among a repo's contents `entries`, sorted. Flat: a `dir`
/// entry is skipped, because a repo holds its boxes at the root.
fn boxes(entries: Vec<Entry>) -> Vec<String> {
    let ext = format!(".{}", crate::vault::store::EXT);
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == "file" && e.name.ends_with(&ext))
        .map(|e| e.name)
        .collect();
    names.sort();
    names
}

pub fn list(repo: &str, token: &str) -> Result<Vec<String>, String> {
    let header = Zeroizing::new(format!("Bearer {token}"));
    let call = agent();
    let mut res = call
        .get(format!("{API}/repos/{repo}/contents/"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(net)?;
    // A repo with no commits answers 404 on its contents root. That is an empty
    // folder, not a missing repo, and it is the state a fresh repo starts in —
    // but a private repo this token cannot see answers that same 404, so the
    // empty folder is confirmed against the repo itself before it is believed.
    if res.status().as_u16() == 404 {
        let repo = call
            .get(format!("{API}/repos/{repo}"))
            .header("Authorization", header.as_str())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(net)?;
        if let Some(why) = denied(repo.status().as_u16(), repo.headers()) {
            return Err(why);
        }
        return Ok(Vec::new());
    }
    if let Some(why) = denied(res.status().as_u16(), res.headers()) {
        return Err(why);
    }
    let raw: Vec<Entry> = res.body_mut().read_json().map_err(net)?;
    Ok(boxes(raw))
}

pub enum Written {
    /// The repo now holds this blob sha.
    Sha(String),
    /// The file moved since this box last read it: the sha the push named is no
    /// longer the one on the branch, so GitHub refused to overwrite it.
    Conflict,
    /// GitHub is refusing writes for this many seconds — the write limit, and
    /// the only place it ever says so.
    Later(u64),
}

/// What a push to a public repo is refused with: nothing of a box goes to one.
pub fn public_refusal() -> String {
    crate::shared::i18n::t("that repo is public — a box needs a private one").to_owned()
}

/// Is `repo` private? A box only ever goes to a private one: the commit subject
/// is fixed (see [`Fingerprint`]), but a public repo still lets anyone list the
/// files in it, their sizes and the dates they changed. Asked of GitHub at the
/// write and never remembered from when the repo was picked, so a repo made
/// public afterwards is caught here too.
fn private(base: &str, repo: &str, token: &str) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Meta {
        #[serde(default)]
        private: bool,
    }

    let header = Zeroizing::new(format!("Bearer {token}"));
    let mut res = agent()
        .get(format!("{base}/repos/{repo}"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(net)?;
    if let Some(why) = denied(res.status().as_u16(), res.headers()) {
        return Err(why);
    }
    let meta: Meta = res.body_mut().read_json().map_err(net)?;
    Ok(meta.private)
}

/// Write the repo's copy of `file`. `sha` is the version being replaced — `None`
/// creates the file, `Some` proves nothing moved since it was read.
pub fn put(
    repo: &str,
    file: &str,
    token: &str,
    bytes: &[u8],
    sha: Option<&str>,
    message: &str,
) -> Result<Written, String> {
    write(API, repo, file, token, bytes, sha, message)
}

/// [`put`] against one API base, so the gate and the write can be tested against
/// a server that is not GitHub's.
fn write(
    base: &str,
    repo: &str,
    file: &str,
    token: &str,
    bytes: &[u8],
    sha: Option<&str>,
    message: &str,
) -> Result<Written, String> {
    // Before the write and before `message` is used: a public repo is refused
    // here, so no bytes of a box ever reach one.
    if !private(base, repo, token)? {
        return Err(public_refusal());
    }

    #[derive(serde::Serialize)]
    struct Body<'a> {
        message: &'a str,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sha: Option<&'a str>,
    }

    let header = Zeroizing::new(format!("Bearer {token}"));
    let body = Body {
        message,
        content: codec::encode(bytes),
        sha,
    };
    let call = agent();
    let mut res = call
        .put(format!("{base}/repos/{repo}/contents/{file}"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        .send_json(&body)
        .map_err(net)?;

    match res.status().as_u16() {
        // A concurrent edit, told apart from every other refusal.
        409 => Ok(Written::Conflict),
        // The write limit, and the seconds to wait for it.
        403 | 429 => Ok(Written::Later(quiet_for(res.headers()))),
        200..=299 => {
            let saved: Saved = res.body_mut().read_json().map_err(net)?;
            Ok(Written::Sha(saved.content.sha))
        }
        status => Err(denied(status, res.headers())
            .unwrap_or_else(|| tf("GitHub refused with HTTP {status}", &[("status", &status)]))),
    }
}

/// Take the repo's copy of `file` out of it. `sha` is the blob the repo holds —
/// the Contents API wants the delete to name the version it means, so a copy
/// that moved in the meantime is not what disappears.
pub fn erase(repo: &str, file: &str, sha: &str, token: &str) -> Result<(), String> {
    erase_at(API, repo, file, sha, token)
}

/// [`erase`] against one API base, so the call can be tested off GitHub.
fn erase_at(base: &str, repo: &str, file: &str, sha: &str, token: &str) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Body<'a> {
        message: &'a str,
        sha: &'a str,
    }

    let header = Zeroizing::new(format!("Bearer {token}"));
    let res = agent()
        .delete(format!("{base}/repos/{repo}/contents/{file}"))
        .header("Authorization", header.as_str())
        .header("Accept", "application/vnd.github+json")
        // A DELETE nominally has no body; the Contents API wants these two
        // fields in one, so the request is built the way ureq documents for it.
        .force_send_body()
        .send_json(&Body {
            message: Fingerprint::SUBJECT,
            sha,
        })
        .map_err(net)?;
    match res.status().as_u16() {
        200..=299 => Ok(()),
        // Already gone: that is what was asked for, and a box that was never
        // pushed has no copy to name in the first place.
        404 => Ok(()),
        status => Err(denied(status, res.headers())
            .unwrap_or_else(|| tf("GitHub refused with HTTP {status}", &[("status", &status)]))),
    }
}

/// How long a settled box waits before its own push. The file is written after
/// 900 ms because a crash may not cost the edits; the commit waits 15 s because
/// GitHub counts commits against an hourly budget, and typing is not a change.
pub const PUSH_QUIET: Duration = Duration::from_secs(15);

pub const PUSH_BACKOFF: Duration = Duration::from_secs(60);

/// How long a repo's own listing is trusted before the view reads it again.
/// Reads are not commits, so this paces against the API's hourly request limit
/// (5000) and not the write budget: one request a minute is nothing.
// ponytail: a plain GET every window. `If-None-Match` with a stored ETag would
// turn the poll into a free 304 if reads ever start to matter.
pub const STALE: Duration = Duration::from_secs(60);

/// Commits GitHub allows per hour: its documented limit on content-generating
/// requests. The boundary this app paces against.
pub const WRITE_LIMIT: usize = 500;

/// This run's spend against that budget. GitHub reports its *primary* limit in
/// the headers and says the secondary one cannot be checked, so pacing is by our
/// own push count and a refusal is the only authoritative answer.
#[derive(Default)]
pub struct Quota {
    sent: Vec<Instant>,
    /// No automatic push before this, because GitHub said so.
    hold: Option<Instant>,
}

impl Quota {
    pub fn record(&mut self, now: Instant) {
        self.sent.push(now);
    }

    pub fn spent(&mut self, now: Instant) -> usize {
        self.sent
            .retain(|at| now.duration_since(*at) < Duration::from_secs(3600));
        self.sent.len()
    }

    /// Stay off the network for `secs`, because a refusal asked for it.
    pub fn hold(&mut self, secs: u64, now: Instant) {
        self.hold = Some(now + Duration::from_secs(secs));
    }

    pub fn held(&self, now: Instant) -> Option<Duration> {
        self.hold
            .and_then(|until| until.checked_duration_since(now))
            .filter(|left| !left.is_zero())
    }
}

/// What a box looked like to a commit: nothing at all.
///
/// A commit subject is written to the repo in plaintext, and a repo is read by
/// whoever holds it — GitHub included. So the subject is one fixed line: not a
/// name, not a count, not which collection moved. A `Fingerprint` only records
/// *that* a version went out, which is all the baseline needs to know.
pub struct Fingerprint;

impl Fingerprint {
    /// The one subject every push carries.
    pub const SUBJECT: &'static str = "pandora: box synced";

    /// The box is the argument because every caller has one; nothing is read off
    /// it, and that is the point — nothing of the box may reach the commit log.
    pub fn of(_vault: &Vault) -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::Login;

    /// GitHub reads the code off this parameter, so its name is the contract —
    /// and GitHub's own complete URI wins when it sends one.
    #[test]
    fn the_sign_in_page_carries_the_code() {
        let bare = "https://github.com/login/device";
        let full = "https://github.com/login/device?user_code=WDJB-MJHT";

        assert_eq!(
            open_uri(None, bare, "WDJB-MJHT"),
            full,
            "nothing from GitHub: the code rides the plain URL"
        );
        assert_eq!(
            open_uri(Some(String::new()), bare, "WDJB-MJHT"),
            full,
            "an empty field is no field"
        );
        assert_eq!(
            open_uri(Some(full.to_owned()), bare, "WDJB-MJHT"),
            full,
            "and GitHub's own page wins when it comes"
        );
    }

    /// The decoder rejects the line wrapping the API adds.
    #[test]
    fn githubs_wrapped_base64_still_decodes() {
        let raw: Vec<u8> = (0..160u16).map(|b| b as u8).collect();
        let encoded = codec::encode(&raw);
        let wrapped = encoded
            .as_bytes()
            .chunks(60)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(wrapped.contains('\n'));
        assert_eq!(payload(&wrapped).unwrap(), raw);
        assert!(payload("not base64").is_err());
    }

    /// A read spells `content` for the base64 of the bytes, a write spells it for
    /// the file object itself, and parsing the write with the read's type is what
    /// made every push fail. What the next write names is the file's own sha.
    #[test]
    fn a_write_reply_is_not_a_read() {
        let json = r#"{"content":{"name":"personal.pandora","path":"personal.pandora",
                        "sha":"b1a2","size":4096,"type":"file"},
                       "commit":{"sha":"c3d4","message":"pandora: box synced"}}"#;
        let saved: Saved = serde_json::from_str(json).unwrap();
        assert_eq!(saved.content.sha, "b1a2");

        assert!(
            serde_json::from_str::<Content>(json).is_err(),
            "a write reply is not the read's shape"
        );
    }

    #[test]
    fn a_contents_listing_keeps_only_the_boxes() {
        let json = r#"[
            {"name":"work.pandora","path":"work.pandora","type":"file","size":4096,
             "sha":"a1","download_url":"https://example.invalid/work.pandora"},
            {"name":"notes.md","type":"file","size":12,"sha":"b2"},
            {"name":"archive","type":"dir","sha":"c3"}
        ]"#;
        let entries: Vec<Entry> = serde_json::from_str(json).unwrap();
        assert_eq!(boxes(entries), ["work.pandora"]);

        assert_eq!(
            boxes(Vec::new()),
            Vec::<String>::new(),
            "an empty repo holds no boxes"
        );
    }

    #[test]
    fn a_refusal_is_worded() {
        let bare = ureq::http::HeaderMap::new();
        assert!(denied(401, &bare).unwrap().contains("sign in again"));
        assert!(denied(404, &bare).unwrap().contains("repo or file"));
        assert!(denied(500, &bare).unwrap().contains("500"));
        assert!(denied(200, &bare).is_none(), "a status that went through");
        assert!(denied(201, &bare).is_none());

        assert_eq!(quiet_for(&bare), 60, "say nothing, wait a minute");
        assert!(
            denied(403, &bare).unwrap().contains("organisation"),
            "a 403 that spent no budget is a permission, not the rate limit"
        );

        let mut spent = ureq::http::HeaderMap::new();
        spent.insert(
            "x-ratelimit-remaining",
            ureq::http::HeaderValue::from_static("0"),
        );
        assert!(denied(403, &spent).unwrap().contains("1 min"));
    }

    /// Another instance's token is not ours: only a different one counts as a
    /// takeover, so the same spent token is never adopted by mistake.
    #[test]
    fn a_takeover_needs_a_different_token() {
        assert_eq!(
            super::takeover("ghr_spent", Some("ghr_other".to_owned())),
            Some("ghr_other".to_owned())
        );
        assert_eq!(
            super::takeover("ghr_spent", Some("ghr_spent".to_owned())),
            None
        );
        assert_eq!(super::takeover("ghr_spent", None), None);
    }

    /// Only a 401 is the session itself being gone; everything else is worth
    /// another try, so nothing else may drop the token.
    #[test]
    fn only_a_dead_token_counts_as_expired() {
        let bare = ureq::http::HeaderMap::new();
        assert_eq!(denied(401, &bare).unwrap(), EXPIRED);
        assert!(expired(EXPIRED));

        assert!(!expired(&denied(403, &bare).unwrap()));
        assert!(!expired(&denied(404, &bare).unwrap()));
        assert!(!expired("ghi"));
    }

    /// A renewal GitHub refused is the session being gone; one that never reached
    /// GitHub is not, and signing the user out over a dropped connection is the
    /// bug this tells apart.
    #[test]
    fn a_refused_renewal_is_not_a_renewal_that_never_landed() {
        assert!(dead(DEAD));

        assert!(!dead(&net("error sending request")));
        assert!(!dead("the renewal was interrupted"));
        assert!(!dead(EXPIRED), "the two walls stay apart");
        assert!(!expired(DEAD));
    }

    /// Both halves of a session come off the endpoint's own answer, and an
    /// expiring one is told from one that never runs out.
    #[test]
    fn a_token_answer_becomes_a_session() {
        let expiring: Token = serde_json::from_str(
            r#"{"access_token":"gho_a","refresh_token":"ghr_b","expires_in":28800}"#,
        )
        .unwrap();
        let session = expiring.into_session();
        assert_eq!(session.access.as_str(), "gho_a");
        assert_eq!(
            session.refresh.as_ref().map(|r| r.as_str()),
            Some("ghr_b"),
            "the refresh token is what renews it"
        );
        assert!(
            session
                .expires_at
                .unwrap()
                .saturating_duration_since(Instant::now())
                > Duration::from_secs(28800 - 60),
            "the deadline is the seconds GitHub sent"
        );

        let lasting: Token =
            serde_json::from_str(r#"{"access_token":"gho_c","token_type":"bearer"}"#).unwrap();
        let session = lasting.into_session();
        assert_eq!(session.access.as_str(), "gho_c");
        assert!(
            session.refresh.is_none() && session.expires_at.is_none(),
            "nothing renews it, so nothing about it runs out"
        );

        // A renewing session that forgot to say when still gets a deadline:
        // otherwise it would be used until GitHub refused it.
        let vague: Token =
            serde_json::from_str(r#"{"access_token":"gho_d","refresh_token":"ghr_e"}"#).unwrap();
        let left = vague
            .into_session()
            .expires_at
            .unwrap()
            .saturating_duration_since(Instant::now());
        assert!(
            left > Duration::from_secs(ACCESS_TTL - 60),
            "left {left:?} of a documented {ACCESS_TTL} s"
        );
    }

    #[test]
    fn the_write_budget_is_a_rolling_hour() {
        let mut quota = Quota::default();
        let now = Instant::now();
        for _ in 0..3 {
            quota.record(now);
        }
        assert_eq!(quota.spent(now), 3);
        assert_eq!(quota.spent(now + Duration::from_secs(3599)), 3);
        assert_eq!(
            quota.spent(now + Duration::from_secs(3601)),
            0,
            "an hour later the hour is empty again"
        );
        assert_eq!(quota.held(now), None);

        quota.hold(120, now);
        assert_eq!(
            quota.held(now + Duration::from_secs(60)),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            quota.held(now + Duration::from_secs(121)),
            None,
            "the hold runs out on its own"
        );
    }

    /// A subject is plaintext in the repo, so it says nothing about the box: not
    /// a name, not a count, and not which record moved. Whatever the box does,
    /// the commit log reads the same.
    #[test]
    fn a_commit_message_never_carries_a_name() {
        let mut vault = Vault::new("Personal");
        let mut bank = Login::new(1);
        bank.id = "bank".to_owned();
        bank.name = "Bank".to_owned();
        let mut git = Login::new(1);
        git.id = "git".to_owned();
        git.name = "GitHub".to_owned();
        vault.logins = vec![bank, git];

        let subject = Fingerprint::SUBJECT;
        assert_eq!(subject, "pandora: box synced");
        assert!(!subject.contains('\n'), "a subject is one line");

        // A rename, an addition, a deletion and an edit: the whole diff a subject
        // used to spell out.
        vault.logins.remove(1);
        let mut netflix = Login::new(2);
        netflix.id = "nflx".to_owned();
        netflix.name = "Netflix".to_owned();
        vault.logins.push(netflix);
        vault.logins[0].touched(3);
        vault.name = "Home".to_owned();

        assert_eq!(
            Fingerprint::SUBJECT,
            subject,
            "what moved does not change what the repo is told"
        );
        for name in ["Personal", "Home", "Bank", "GitHub", "Netflix"] {
            assert!(
                !subject.contains(name),
                "{name:?} reached the commit log: {subject}"
            );
        }
        let _ = Fingerprint::of(&vault);
    }

    /// One request off `server`, read whole — the head, then the body its own
    /// `content-length` announces — and answered with `body` as JSON. The request
    /// comes back as text, so a test can read the line and the fields.
    fn serve(server: &std::net::TcpListener, body: &[u8]) -> String {
        use std::io::{Read, Write};

        let (mut sock, _) = server.accept().unwrap();
        let mut buf = vec![0u8; 8192];
        let mut request: Vec<u8> = Vec::new();
        let head_at = loop {
            let n = sock.read(&mut buf).unwrap();
            request.extend_from_slice(&buf[..n]);
            if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(&request[..head_at]).to_lowercase();
        let want = head
            .split("content-length: ")
            .nth(1)
            .and_then(|rest| rest.split('\r').next())
            .and_then(|len| len.parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() - head_at < want {
            let n = sock.read(&mut buf).unwrap();
            request.extend_from_slice(&buf[..n]);
        }
        let answer = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(answer.as_bytes());
        let _ = sock.write_all(body);
        String::from_utf8_lossy(&request).into_owned()
    }

    /// A box only ever goes to a private repo: the write asks GitHub first, and a
    /// public answer stops it before a byte of the box is on its way.
    #[test]
    fn a_public_repo_is_never_written_to() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = listener.try_clone().unwrap();
        let answer = std::thread::spawn(move || {
            serve(
                &server,
                br#"{"private":false,"full_name":"octocat/public"}"#,
            )
        });

        let refused = write(
            &base,
            "octocat/public",
            "work.pandora",
            "t",
            b"box",
            None,
            "subject",
        );
        let request = answer.join().unwrap();

        assert!(
            request.starts_with("GET /repos/octocat/public "),
            "the write asks before it writes: {request}"
        );
        let why = refused.err().expect("a public repo takes no write");
        assert_eq!(why, public_refusal());
        assert!(why.contains("private"), "{why}");

        // And nothing else was asked for: the write never went out at all.
        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_err(),
            "a second request came after the answer"
        );
    }

    /// A private answer lets the write through — asked again on every push, so a
    /// repo made public after it was picked is caught on the next one.
    #[test]
    fn a_private_repo_takes_the_write() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let answer = std::thread::spawn(move || {
            [
                serve(&listener, br#"{"private":true}"#),
                serve(&listener, br#"{"content":{"sha":"new"}}"#),
            ]
        });

        let written = write(
            &base,
            "octocat/private",
            "work.pandora",
            "t",
            b"box",
            None,
            "subject",
        );
        let seen = answer.join().unwrap();

        assert!(
            seen[0].starts_with("GET /repos/octocat/private "),
            "{seen:?}"
        );
        assert!(
            seen[1].starts_with("PUT /repos/octocat/private/contents/work.pandora "),
            "the write is the second call: {seen:?}"
        );
        match written {
            Ok(Written::Sha(sha)) => assert_eq!(sha, "new"),
            Ok(Written::Conflict) => panic!("the repo did not move"),
            Ok(Written::Later(secs)) => panic!("refused for {secs} s"),
            Err(e) => panic!("the private write landed: {e}"),
        }
    }

    /// The repo's copy leaves by a delete that names the version it means, so a
    /// copy that moved in the meantime is not what disappears.
    #[test]
    fn erasing_the_repo_copy_names_the_version() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let answer = std::thread::spawn(move || serve(&listener, b"{}"));

        let asked = erase_at(&base, "octocat/boxes", "work.pandora", "abc123", "t");
        let request = answer.join().unwrap();

        assert!(
            asked.is_ok(),
            "a delete that landed is a no-op to the caller"
        );
        assert!(
            request.starts_with("DELETE /repos/octocat/boxes/contents/work.pandora "),
            "{request}"
        );
        assert!(
            request.contains("\"sha\"") && request.contains("abc123"),
            "the version the repo holds is what the delete names: {request}"
        );
    }

    /// A box small enough for the JSON reply is read straight out of it: one request,
    /// and the blob API is never asked.
    #[test]
    fn a_small_box_comes_back_in_the_contents_reply() {
        use std::net::TcpListener;

        let body = format!(
            r#"{{"content":"{}","sha":"abc123"}}"#,
            codec::encode(b"a box")
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = listener.try_clone().unwrap();
        let answer = std::thread::spawn(move || serve(&server, body.as_bytes()));

        let read = get_at(&base, "octocat/boxes", "work.pandora", "t").expect("read");
        let request = answer.join().unwrap();

        assert_eq!(read.bytes, b"a box");
        assert_eq!(read.sha, "abc123");
        assert!(
            request.starts_with("GET /repos/octocat/boxes/contents/work.pandora "),
            "{request}"
        );
    }

    /// Past 1 MB the Contents API hands back the sha and no content. The bytes come
    /// from the Git Blobs API under that same sha, so a box bigger than the JSON
    /// reply is read instead of being refused — the ceiling was always the reply,
    /// never the file.
    #[test]
    fn a_box_over_a_megabyte_is_read_through_the_blob_api() {
        use std::net::TcpListener;

        let big = vec![b'x'; 1024 * 1024 + 4096];
        let contents = br#"{"content":"","sha":"abc123"}"#.to_vec();
        let blob = format!(
            r#"{{"content":"{}","encoding":"base64","size":{}}}"#,
            codec::encode(&big),
            big.len()
        );

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = listener.try_clone().unwrap();
        let answer = std::thread::spawn(move || {
            let first = serve(&server, &contents);
            let second = serve(&server, blob.as_bytes());
            (first, second)
        });

        let read = get_at(&base, "octocat/boxes", "work.pandora", "t").expect("read");
        let (asked, asked_blob) = answer.join().unwrap();

        assert_eq!(read.bytes, big, "the bytes are the box that was pushed");
        assert_eq!(read.sha, "abc123", "the sha is still the file's own");
        assert!(
            asked.starts_with("GET /repos/octocat/boxes/contents/work.pandora "),
            "{asked}"
        );
        assert!(
            asked_blob.starts_with("GET /repos/octocat/boxes/git/blobs/abc123 "),
            "the bytes come from the blob API: {asked_blob}"
        );
    }

    /// A blob the API will not hand back is not a box: `encoding: "none"` past the
    /// 100 MB it documents, and anything that is not base64 is refused with words a
    /// user can act on rather than a decode error.
    #[test]
    fn a_blob_the_api_will_not_decode_is_refused() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = listener.try_clone().unwrap();
        let answer =
            std::thread::spawn(move || serve(&server, br#"{"content":"","encoding":"none"}"#));

        let refused = blob_at(&base, "octocat/boxes", "abc123", "t");
        answer.join().unwrap();

        assert!(
            refused.as_ref().is_err_and(|why| why.contains("too large")),
            "{refused:?}"
        );
    }

    /// A refusal must arrive as a response, or `retry-after` is lost.
    #[test]
    fn a_refusal_arrives_with_its_status_and_headers() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let answer = std::thread::spawn(move || {
            let (mut sock, _) = server.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(
                b"HTTP/1.1 403 Forbidden\r\nretry-after: 90\r\ncontent-length: 0\r\n\r\n",
            );
        });

        let res = agent()
            .put(format!("http://127.0.0.1:{port}/x"))
            .send_json(serde_json::json!({"a": 1}))
            .expect("a refusal is a response, not a transport error");

        assert_eq!(res.status().as_u16(), 403);
        assert_eq!(quiet_for(res.headers()), 90, "github said how long");
        assert!(denied(res.status().as_u16(), res.headers())
            .unwrap()
            .contains("2 min"));
        answer.join().unwrap();
    }
}
