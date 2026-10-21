# Pandora

![Pandora — your secrets, boxed.](assets/banner.png)

> Your secrets, boxed.

Pandora keeps your secrets in a single file you own. There is no account, no
server and no sync service in the middle: the box is a `.pandora` file, encrypted
with a key derived from your master password, that you can copy, back up, put on a
USB stick or mirror to a private GitHub repo. The app is a native desktop program
(egui/eframe) and the same code runs on Android.

<p align="center">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg">
  <img alt="Linux, macOS, Windows, Android" src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android-lightgrey.svg">
  <img alt="Rust" src="https://img.shields.io/badge/built%20with-Rust-orange.svg">
</p>

- **One file.** Everything — logins, folders, notes, documents, TOTP seeds and the
  trash — lives inside the encrypted payload. Nothing else is needed beside it:
  your settings live in the app data directory, not in the box.
- **Offline by default.** The only network request the app can make is to GitHub,
  and only after you add a repo yourself.
- **No lock-in.** An open container with a JSON manifest, plain Rust, and a
  plaintext CSV import/export for when you want to leave.

## Features

### Passwords

- Logins with name, username, password, notes and **one or more URLs**.
- Per-URL matching rules (`domain`, `host`, `prefix`, `never`) so a login only
  fills the sites it should. The default is the registrable domain from the Public
  Suffix List, so `mail.google.com` matches `accounts.google.com` but
  `github.com` never matches `github.com.evil.com`.
- Folders, drag to move, list or generated folders view (grouped by site or
  username), and a `Ctrl+K` search over the whole box — names, users, addresses,
  notes, folder names, note bodies and code names. Passwords, seeds and file
  bytes are never searched.
- Autosave after 900 ms of quiet, atomic writes, and a one-slot backup of the
  previous version (`<box>.pandora.bak`).

### Files

- A note is text; a document is bytes. They are different types, so a note cannot
  carry a file and a file cannot carry text.
- Preview images (PNG, JPEG, WebP), PDF pages and text/code without leaving the
  app. Audio (MP3, WAV, FLAC, OGG) can be played. Video and archives get a card,
  not a player — save them out to open them.
- Drag and drop onto the window on the desktop; the phone's own file picker on
  Android. Documents are streamed into encrypted chunks, so there is no per-file
  size ceiling; previews stop at 64 MiB and point you at Save.

### Codes

- TOTP seeds in Base32 or `otpauth://` form. SHA-1, SHA-256 and SHA-512, 6–8
  digits and a custom period.
- Read a seed from a QR code: from a screenshot on the clipboard or a file on the
  desktop, and from the photo picker or the live full-screen scanner on a phone.
  The reader is ZXing's (`rxing`), because the hard case is a photograph of a
  screen, not a clean render.

### Around the box

- **Password generator** with length and character classes, reachable with
  `Ctrl+3` even when no box is open.
- **Multiple boxes** open in tabs, with a recents screen that, when empty,
  points to the local files and GitHub repos; each box has its own master
  password.
- **Trash and undo.** Deleting a record is reversible (`Ctrl+Z`) until the trash
  is emptied; only then are a file's bytes erased.
- **CSV import/export.** Import reads the column aliases KeePassXC, KeePass and
  Bitwarden write; export writes every login, note and code as plaintext
  (documents are skipped) after a confirmation that names the file and offers a
  neutral name instead.
- **GitHub mirror.** The box is pushed as one opaque encrypted blob through the
  Contents API. The local file is the source; the repo is a copy.
- **Browser extension.** A Chrome/Firefox WebExtension fills logins through a
  native-messaging host, and fills a one-time code from the Codes pane into a field
  that asks for one. It never reads the vault and never runs the crypto.
- **Fingerprint unlock** on Android: the device's reader unlocks the platform
  keystore, and the keystore holds a sealed copy of the box's derived key.
- **Settings:** the box's own page — master password, idle lock, clipboard
  countdown, unlock attempt limits, browser access, the trash — and a general one
  for the app itself: English or Spanish, and dark or light.

## How it works

```
desktop / phone ──────▶ Pandora app ──────▶ one encrypted .pandora file
                             │
                             ├───────────▶ GitHub repo (optional mirror)
                             │
browser extension ──▶ native host ──▶ local bridge (the app holds the key)
```

With no app listening, the host opens the box file itself. Letting a browser
unlock it that way is the Settings → Browser option *Let a browser open a box on
its own*, off by default; the extension otherwise says to open the box in the
app.

The crate is split so the vault core has no UI and no platform knowledge:

```
src/
  vault/       format, crypto, matching, search, TOTP — no egui
  shared/      i18n, theme, responsive UI helpers, the Android shim
  features/    one directory per capability: entries, drive, codes,
               boxes, auth, sync, settings, find, shell…
  bridge/      the local socket the browser host relays to
  host/        the native-messaging host's behaviour
  app.rs       the composition root: the loop and cross-cutting state
  bin/         pandora-native-host
```

`AGENTS.md` documents the ownership rules, the feature-first layout and the
platform split in detail. `android/README.md` covers the phone, and
`../extension/README.md` the browser side.

## Security

- **Key derivation:** Argon2id, 64 MiB of memory, 3 passes, 4 lanes, 16-byte
  salt. Derivation runs on a worker thread, never the UI thread.
- **Encryption:** XChaCha20-Poly1305 with a random 24-byte nonce, for the
  manifest and for each 1 MiB chunk. Chunk AAD binds the box id, the record id
  and kind, the chunk index and the plaintext length.
- **Header binding:** the header is authenticated as AEAD associated data, so
  the KDF parameters cannot be changed under the ciphertext. Only v4 boxes are
  read; an older container is refused rather than migrated. The manifest
  authenticates the header and the chunks stay outside it.
- **Size padding:** each chunk pads its last plaintext block to 4 KiB, and the
  whole file is rounded to a 4 KiB block with a 64 KiB floor, so the file size
  does not reveal how much is inside and a small box is never its own few
  hundred bytes.
- **Atomic writes:** sibling temp file → `fsync` → rename → directory `fsync`. A
  half-written box is never left behind, and the previous version is kept in one
  `<box>.pandora.bak` slot — a password change drops it, because it is sealed
  with the key that change replaced.
- **Keyfile (optional):** a BLAKE2b-512 digest of the file is appended to the
  password before derivation, so the file alone is never a key. The path is never
  stored in the box, and the box does not say whether one was used either: the
  app derives with the keyfile when one is handed over and the AEAD tag says
  which key the box owns. Losing it loses the vault.
- **Unlock limits:** failed attempts are counted per box file, with an optional
  delay and a configurable maximum. A successful unlock forgets the failures.
- **In memory:** the master password is used only for derivation and dropped; the
  derived key and secret fields are `Zeroizing`. Record types hand-write their
  `Debug` so a panic cannot print one.
- **Clipboard:** a copied password is cleared after a countdown (30 s by default).
  The app keeps a BLAKE2 fingerprint, not the text, so something you copied
  afterwards is left alone. `0` never clears.
- **Idle lock:** after 15 minutes by default, and on the OS locking or suspending,
  every open box is flushed and locked. `0` disables it.
- **One app per data directory:** a `pandora.lock` file holds the settings, the
  recents and the vault against a second instance.

### What this does not protect against

Pandora is not audited. An unlocked box is plaintext in process memory, so it
does not defend against malware running as you, a debugger, root, or a memory
dump. The browser bridge socket is protected by being user-private — same-user
malware can claim any caller id, exactly as it can against KeePassXC. Fingerprint
unlock is only as strong as the platform keystore. And the CSV export is
**plaintext by design**; the app warns you before writing it.

[`SECURITY.md`](SECURITY.md) is the long form: who is assumed honest, what is out
of scope and why, and how to report something that looks wrong.

## The vault file

A box is one file. v4 writes a fixed 4 KiB header (`PANDORA4\n` plus JSON with
the KDF, a random box id and the manifest's location), 1 MiB encrypted chunks for
documents and notes, and an encrypted manifest at the end. This version only
opens v4; an older JSON container is refused rather than migrated.

The decrypted manifest holds four collections — `folders`, `logins`, `drive`,
`otps` — plus `trash`, the records deleted but not erased. A drive entry is a
tagged `Entry`: a `note` or a `file`. Records are addressed by id, so operations
that do not care about the kind (select, drag, move, delete, export) never need
to know which collection a record came from.

A file's or note's bytes live in chunks outside the manifest; only the manifest
is read when a box opens. A document is decrypted when it is previewed or
exported, one chunk at a time, so the box does not carry every file in memory.
Files picked or dropped on the desktop stay where they are until the save
chunks them; a phone picker copy is staged, chunked and then removed.

## Where things live

`prefs.json`, `recent.json` and `biometric.json` sit in the app data directory,
**never inside a box**:

| Platform | Data directory |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/pandora/` |
| macOS | `~/Library/Application Support/pandora/` |
| Windows | `%LOCALAPPDATA%\pandora\` |
| Android | the app's private directory, handed over by the activity |

```
pandora/
  vault.pandora          default box (yours can live anywhere)
  prefs.json             settings, sources, remotes, attempt counters
  recent.json            boxes the Unlock screen offers
  biometric.json         sealed per-box keys (ciphertext only)
  pandora.lock           single-instance lock
  sources/               working copies of boxes pulled from a repo
```

Beside a box you may see `<box>.pandora.bak` (the previous version) and
`<box>.pandora.<uuid>.tmp` only while a write is in flight.

## Install

### Desktop

Pandora builds from source with a stable Rust toolchain (edition 2021):

```sh
cargo run --release
```

On Linux the audio preview links against ALSA, so the development headers are
needed:

```sh
# Debian / Ubuntu
sudo apt install libasound2-dev
# Fedora
sudo dnf install alsa-lib-devel
```

macOS and Windows need only the Rust toolchain. On Linux the app runs native
Wayland when the session offers it; set `PANDORA_X11=1` to force the X11 backend
if you need drag-and-drop from the file manager, since winit 0.30 only emits file
drops from X11.

A `.desktop` file and the hicolor icons for a manual install live under
`assets/app/linux/`; macOS (`.icns`) and Windows (`.ico`) icons are next to them.
To package the release build locally:

```sh
scripts/appimage.sh        # Linux: target/appimage/Pandora-<version>-x86_64.AppImage
scripts/package-macos.sh   # macOS: universal .app, .dmg and .zip
```

GitHub sign-in is compiled in from `PANDORA_GITHUB_CLIENT_ID` (set in
`.cargo/config.toml`, which is not committed). A build without it refuses the
GitHub flow as unconfigured; local boxes are unaffected.

### Android

The phone build needs the Android SDK, an NDK and a JDK 17–25. One command builds
the Rust `.so`, packages the release APK and optionally installs it:

```sh
ANDROID_HOME=$HOME/Android/Sdk JAVA_HOME=/path/to/jdk-21 scripts/android-apk.sh install
```

See [`android/README.md`](android/README.md) for minSdk, the arm64-only packaging
and the Rust-side details.

### Browser extension

The extension lives in the sibling `extension/` folder (see
`../extension/README.md`). It installs the native host,
writes the browser manifests and assembles the unpacked Chrome/Firefox builds:

```sh
../extension/install.sh
```

## Use

1. **Create a box** with a master password (12 characters minimum, with a
   strength meter) and, if you want, a keyfile.
2. **Add records** with `Ctrl+N`. A login gets one or more URLs; a note, file or
   code lives in its own pane.
3. **Search** with `Ctrl+K` for anything in the box, or `Ctrl+F` to focus the
   current list's field.
4. **Lock** with `Ctrl+L`, or let the idle timer do it. Unlock with the password,
   the keyfile if it has one, or a fingerprint on a phone that supports it.
5. **Mirror** the box to GitHub from Settings → GitHub if you want an off-machine
   copy. Reads and pushes happen on worker threads; the UI never waits.

### Keyboard

| Shortcut | Action |
| --- | --- |
| `Ctrl+S` | Save now (autosave already runs) |
| `Ctrl+L` | Lock the box |
| `Ctrl+W` | Close the box |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous box |
| `Ctrl+1` | Passwords |
| `Ctrl+2` | Files |
| `Ctrl+5` | Codes |
| `Ctrl+4` | Settings |
| `Ctrl+3` | Password generator |
| `Ctrl+K` | Search the whole box |
| `Ctrl+F` | Focus the search field |
| `Ctrl+N` | New record |
| `F2` | Rename folder |
| `Delete` | Move the selected record to the trash |
| `Ctrl+Z` | Undo the last delete |
| `Ctrl+C` / `Ctrl+B` | Copy the selected password / username |
| `Escape` | Clear the selection, or close the dialog |
| Browser Back | Go back (also the Android back button) |

Below 620 px wide the layout collapses to a single column, which is what the
phone uses.

### Environment

| Variable | Effect |
| --- | --- |
| `PANDORA_X11=1` | Force winit's X11 backend on Linux, which is what brings back drag-and-drop. |
| `PANDORA_DATA_DIR` | Data directory for the native host. Used by the flatpak browser install; the app itself always uses the platform's own. |
| `PANDORA_BRIDGE_SOCKET` | Override the bridge socket path (`$XDG_RUNTIME_DIR/pandora-bridge.sock` by default). |
| `PANDORA_GITHUB_CLIENT_ID` | Build-time OAuth client id for the GitHub mirror. Without it, sign-in is refused. |

## Development

```sh
cargo test                        # full suite, ~25 s
cargo test vault::store           # one module
cargo test create_open_roundtrip  # one test
cargo fmt
cargo clippy --all-targets
```

### Releases

`.github/workflows/release.yml` builds every shipped platform:

| Platform | Artifact |
| --- | --- |
| Linux | `Pandora-<version>-x86_64.AppImage` |
| Windows | `Pandora-<version>-windows-x86_64.zip` (portable, unsigned) |
| macOS | `Pandora-<version>-macos-universal.dmg` and `.zip` (ad-hoc signed, not notarized) |
| Android | `Pandora-<version>-android-arm64.apk` (debug key, not store-ready) |

The Linux job runs on Ubuntu 22.04 on purpose: the AppImage inherits the
runner's glibc floor, and 2.35 still opens on Debian 12 and Ubuntu 22.04.

A `vX.Y.Z` tag runs every job and publishes the artifacts plus `SHA256SUMS` as a
GitHub Release. PRs do not build packages: `check.yml` is the PR gate, and it
typechecks Linux, Windows and macOS.

A manual `workflow_dispatch` accepts `jobs` (`all` or a comma-separated subset of
`linux,windows,macos,android`) and an optional `tag_name`. Without `tag_name` it
only builds artifacts; with it, it publishes that tag. The version comes from
`Cargo.toml`, and the workflow refuses a tag or Android `versionName` that
disagrees.

To cut a release, bump `version` in `Cargo.toml` and `versionName` in
`android/app/build.gradle`, commit, and push the tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The build reads the public OAuth client id from the repository variable
`PANDORA_GITHUB_CLIENT_ID` (Settings → Secrets and variables → Actions →
Variables). Without it the build still succeeds, but the GitHub mirror is
unconfigured, exactly like a local build without `.cargo/config.toml`.

To run the Linux job locally with `act` and Podman, expose Podman's Docker API in
one terminal:

```sh
podman system service --time=0 "unix://${XDG_RUNTIME_DIR}/podman/podman.sock"
```

and run the workflow from another:

```sh
export DOCKER_HOST="unix://${XDG_RUNTIME_DIR}/podman/podman.sock"
act workflow_dispatch -W .github/workflows/release.yml -j linux \
  -P ubuntu-22.04=catthehacker/ubuntu:act-22.04 \
  -P ubuntu-latest=catthehacker/ubuntu:act-latest \
  --container-architecture linux/amd64 \
  --artifact-server-path /tmp/act-artifacts \
  --var PANDORA_GITHUB_CLIENT_ID=your-client-id
```

`act` only runs the Linux job: Windows and macOS need their own runners, and the
Android job wants the SDK/NDK. `-j linux` stops before `publish`, which needs a
`GITHUB_TOKEN` with `contents: write`. The uploaded artifact is left under
`/tmp/act-artifacts`; the action's working tree is discarded with the container.

`features::shell::screen::probe_rows` is a debug probe, ignored on purpose: it
prints the shapes the top of the list paints and asserts nothing. `cargo test` is
green without it; run it when the question is what the frame actually drew:

```sh
cargo test -- --ignored probe_rows --nocapture
```

There is also a headless resize benchmark that drives the real `App::run_ui`:

```sh
XDG_RUNTIME_DIR=/tmp/pandora-bench \
  cargo run --release --example bench_resize -- 1000 120
```

Layout tests run egui headless and assert on painted shapes through
`shared::ui::shot`. Read [`AGENTS.md`](AGENTS.md) before changing architecture:
it is the project's contract for the feature-first tree, the vault format, the
platform split and the security rules.

## Limitations

- iOS is not wired up yet; the icon assets exist, the entry point does not.
- GitHub is the only mirror provider. A box is read through the Contents API, and
  past 1 MB through the Git Blobs API under the same sha; GitHub caps a blob at
  100 MB, so a box carrying a very large attachment may be too big for GitHub;
  local storage itself has no per-file ceiling.
- A box that needs a keyfile cannot be opened from the browser.
- No video preview, no auto-type, no server-side sync, no plugin API.
- The vault format is not frozen: v4 is the only format this version reads and
  writes; an older container is refused, and a future version may add fields
  (persisted fields must be optional).
- Android packages only `arm64-v8a`, which is every phone that runs minSdk 29.

## License

MIT. See [`LICENSE`](LICENSE).

## Credits

Pandora stands on a lot of good Rust:

- [egui / eframe](https://github.com/emilk/egui) for the immediate-mode UI, glow
  on the desktop (wgpu is the fallback) and glow on the phone.
- [Argon2](https://github.com/RustCrypto/password-hashes) and
  [XChaCha20-Poly1305](https://github.com/RustCrypto/AEADs).
- [rxing](https://github.com/rxing-core/rxing) (ZXing) for QR reading.
- [hayro](https://github.com/typst/hayro) for PDF rasterization in pure Rust,
  and [rodio](https://github.com/RustAudio/rodio) for audio playback.
- [Lucide](https://lucide.dev) for the UI icons (ISC) and
  [simple-icons](https://simpleicons.org) for the brand marks (CC0).
- The Public Suffix List via [`psl2`](https://crates.io/crates/psl2).
