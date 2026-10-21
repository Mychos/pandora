# Pandora

![Pandora — your secrets, boxed.](assets/banner.png)

> Your secrets, boxed.

Pandora keeps your logins, notes, files and one-time codes in **one encrypted file
you own**. No account, no server, no sync service. The box is a `.pandora` file:
copy it, back it up, put it on a stick.

**GitHub built in.** Point it at a private repo of yours and Pandora keeps that copy
current — one encrypted blob, no git, nothing readable on GitHub's side. The file on
your disk stays the source.

Native desktop (egui/eframe), same code on Android.

<p align="center">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg">
  <img alt="Linux, macOS, Windows, Android" src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android-lightgrey.svg">
  <img alt="Rust" src="https://img.shields.io/badge/built%20with-Rust-orange.svg">
</p>

## Why Pandora

Every other password manager puts your secrets on someone else's computer. Pandora
does not.

- **Yours.** One file. Uninstall the app and it still opens in the next version.
- **Offline.** Works unplugged, measures nothing. Two requests exist, both to
  GitHub — the mirror you add and the update check — and both switch off.
- **No lock-in.** JSON manifest, plain Rust, MIT, CSV export in the shape those
  managers already import.
- **Boring crypto.** Argon2id, XChaCha20-Poly1305, the Public Suffix List. Nothing
  homegrown.
- **Honest.** Not audited, plaintext in memory while unlocked, plaintext CSV export.
  All of it is in [`SECURITY.md`](SECURITY.md).
- **Undo.** Autosave, atomic writes, one backup beside the box, a trash and `Ctrl+Z`.

**Not for teams.** No sharing, roles, admin console or account recovery — there is no
account. Need those? Use a hosted manager.

## First questions

**Where does the file go?** Anywhere. Each box can live in its own folder.

**Forgot the master password?** Nothing can reset it. No recovery key, no back door.
Write it down, or use a keyfile.

**Lost the file?** Then you lost that copy. Back it up, or mirror it to a private
GitHub repo.

**What lives outside the box?** Settings, recents and the sealed fingerprint keys —
never vault contents:

| Platform | Data directory |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/pandora/` |
| macOS | `~/Library/Application Support/pandora/` |
| Windows | `%LOCALAPPDATA%\pandora\` |
| Android | the app's private directory, handed over by the activity |

**Can I get my data back out?** Yes: CSV for logins, notes and codes, and documents
through Save. Both are plaintext and both warn you first.

## Features

### Passwords

- Logins with a name, username, password, notes and **one or more URLs**.
- Each URL carries a rule — `domain` (default), `host`, `prefix`, `never`. So
  `mail.google.com` fills `accounts.google.com`, and `github.com` never fills
  `github.com.evil.com`.
- Folders, drag to move, grouped views, and `Ctrl+K` over the whole box: names,
  users, addresses, notes, folders, note bodies, code names. Never passwords, seeds
  or file bytes.

### Files

- A note is text, a document is bytes. Different types, so neither can carry the
  other.
- Images, PDF pages, text and code preview in place. Audio plays. Video and archives
  are save-only.
- Streamed into encrypted chunks, so no per-file ceiling. Previews stop at 64 MiB.

### Codes

- TOTP in Base32 or `otpauth://`, SHA-1/256/512, 6–8 digits, custom period.
- A seed can come from a QR: clipboard, file, photo picker, or the live scanner on a
  phone (ZXing, via `rxing`).

### Around the box

- **Password generator** with length and character classes, on `Ctrl+3`.
- **Multiple boxes** in tabs, each with its own master password.
- **Trash and undo.** Deleting is reversible until the trash is emptied; only then
  are a document's bytes erased.
- **Autosave** after 900 ms of quiet, atomic writes, one `.bak` beside the box.
- **CSV import/export**, reading KeePassXC, KeePass and Bitwarden aliases.
- **GitHub mirror.** One encrypted blob through the Contents API. Private repos
  only, and one made public later is refused on the next push.
- **Browser extension.** Chrome and Firefox, filling logins and codes through a
  native host. It never reads the vault and never runs the crypto.
- **Autofill on Android.** The phone's own fill panel, in any app whose login form
  is a WebView: the entries claiming the page, one row each, and the codes for a
  field that asks for one. It reads the box open in the app and nothing else.
- **Updates.** A newer release is offered as a strip; the click downloads it, checks
  the published SHA-256 and swaps the file. Nothing installs without that click.
- **Fingerprint unlock** on Android, sealed in the platform keystore.
- **Settings:** one page per box, one for the app (English or Spanish, dark or
  light).

## How it works

```
desktop / phone ──────▶ Pandora app ──────▶ one encrypted .pandora file
                             │
                             ├───────────▶ GitHub repo (optional mirror)
                             │
browser extension ──▶ native host ──▶ local bridge (the app holds the key)

phone's fill panel ──▶ the app's copy of the open box (no socket, no second file)
```

The app holds the key while a box is unlocked. The browser asks a small native host,
which relays to the app — or opens the box itself, if you allowed that in Settings →
Browser. Android's fill panel cannot do that: it may be started with no activity at
all, so the app leaves it a copy of the open box instead, dropped the moment the box
locks.

A box is a 4 KiB header, 1 MiB encrypted chunks and an encrypted manifest. Only v4
opens; an older container is refused rather than migrated. Byte layout in
[`AGENTS.md`](AGENTS.md#vault-format).

The vault core has no UI and no platform knowledge, so desktop and phone run the same
code. [`AGENTS.md`](AGENTS.md) maps the tree; the phone has
[`android/README.md`](android/README.md) and the browser side
[`../extension/README.md`](../extension/README.md).

## Security

- **Key.** Argon2id, 64 MiB, 3 passes, 4 lanes, derived off the UI thread.
- **Cipher.** XChaCha20-Poly1305 with a random nonce, in 1 MiB chunks bound to the
  box, the record and the position.
- **Size.** Padded to 4 KiB blocks with a 64 KiB floor, so a one-login box is not
  visibly smaller than a hundred.
- **Keyfile.** Optional: a BLAKE2b digest of a file is mixed into the password. Lose
  it and the vault is gone.
- **Memory and clipboard.** Secrets are `Zeroizing`. A copied password clears after a
  countdown, and an idle box locks after 15 minutes.
- **Limits.** Not audited. Plaintext in process memory while unlocked, so it does not
  stop malware running as you, a debugger, root or a memory dump. Fingerprint unlock
  is as strong as the platform keystore. The CSV export is plaintext by design.

The long form is [`SECURITY.md`](SECURITY.md).

## Install

**Desktop.** Stable Rust (edition 2021):

```sh
cargo run --release
```

Linux needs the ALSA headers (`libasound2-dev`, `alsa-lib-devel`). macOS and Windows
need only the toolchain. Wayland is used when the session offers it; `PANDORA_X11=1`
forces X11, which is what drag-and-drop from the file manager needs. Desktop file and
icons under `assets/app/linux/`; `scripts/appimage.sh` and `scripts/package-macos.sh`
package a release build.

Prebuilt: AppImage on Linux, portable zip and bare `.exe` on Windows, universal
`.dmg`/`.zip` on macOS. The in-app updater installs them.

**Android.** SDK, NDK and a JDK 17–25:

```sh
ANDROID_HOME=$HOME/Android/Sdk JAVA_HOME=/path/to/jdk-21 scripts/android-apk.sh install
```

`arm64-v8a` only, which is every phone on minSdk 29. See
[`android/README.md`](android/README.md).

**Browser extension.** Lives next door and installs the native host:

```sh
../extension/install.sh
```

## Use

1. **Create a box** with a master password (12 characters minimum) and, if you want,
   a keyfile.
2. **Add records** with `Ctrl+N`.
3. **Search** with `Ctrl+K`, or `Ctrl+F` for the current list.
4. **Lock** with `Ctrl+L`, or let the idle timer do it.
5. **Mirror** to GitHub from Settings → GitHub. Reads and pushes run off the UI
   thread.

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

Below 620 px the layout collapses to one column, which is what the phone uses.

### Environment

| Variable | Effect |
| --- | --- |
| `PANDORA_X11=1` | Force winit's X11 backend on Linux, which is what brings back drag-and-drop. |
| `PANDORA_DATA_DIR` | Data directory for the native host. Used by the flatpak browser install; the app itself always uses the platform's own. |
| `PANDORA_BRIDGE_SOCKET` | Override the bridge socket path (`$XDG_RUNTIME_DIR/pandora-bridge.sock` by default). |
| `PANDORA_GITHUB_CLIENT_ID` | Build-time OAuth client id for the GitHub mirror. Without it, sign-in is refused. |

## Development

```sh
cargo run                         # the GUI; needs a display
cargo test                        # full suite, ~25 s
cargo test vault::store           # one module
cargo test create_open_roundtrip  # one test
cargo fmt
cargo clippy --all-targets
```

GitHub sign-in is compiled in from `PANDORA_GITHUB_CLIENT_ID` (set in
`.cargo/config.toml`, not committed); without it the GitHub flow is refused and local
boxes are unaffected.

Releases — workflow, artifacts, the Android signing key, `act` — are in
[`AGENTS.md`](AGENTS.md#releases). Read that file before changing architecture.

## Limitations

- iOS is not wired up: the icons exist, the entry point does not.
- Android autofill answers for WebViews only: a native app has no URL, so no stored
  entry claims it, and Chrome keeps its own forms out of the framework entirely.
  Codes are offered in any app, unfiltered by site.
- GitHub is the only mirror, and caps a blob at 100 MB; a box with a huge attachment
  may not fit. Local storage has no per-file ceiling.
- A keyfile box cannot be opened from the browser.
- No video preview, no auto-type, no server-side sync, no plugin API.
- The format is not frozen: v4 is all this version reads and writes, and a future
  version may add fields (they must be optional).
- Android packages only `arm64-v8a`.

## License

MIT. See [`LICENSE`](LICENSE).

## Credits

- [egui / eframe](https://github.com/emilk/egui) — the immediate-mode UI.
- [Argon2](https://github.com/RustCrypto/password-hashes) and
  [XChaCha20-Poly1305](https://github.com/RustCrypto/AEADs) — the crypto.
- [rxing](https://github.com/rxing-core/rxing) (ZXing) — QR reading.
- [hayro](https://github.com/typst/hayro) — PDF rasterization; [rodio](https://github.com/RustAudio/rodio)
  — audio.
- [Lucide](https://lucide.dev) (ISC) and [simple-icons](https://simpleicons.org)
  (CC0) — icons and brand marks.
- [`psl2`](https://crates.io/crates/psl2) — the Public Suffix List.
