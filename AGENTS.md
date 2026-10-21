# AGENTS.md

Pandora — offline secrets vault stored in one portable encrypted file. Single Rust
crate (no workspace): egui/eframe GUI — Glow on the desktop by default, with wgpu
as the fallback, and glow on the phone, where it is 1.2 MB of APK smaller. Android
is wired up (`android/`, see its README); iOS is not.

A Linux build needs the ALSA headers the audio preview links against
(`libasound2-dev` on Debian/Ubuntu, `alsa-lib-devel` on Fedora).

## Commands

```bash
cargo run                        # GUI; needs a display (Glow by default)
PANDORA_RENDERER=wgpu cargo run  # desktop wgpu fallback
cargo run --no-default-features  # wgpu-only build, no Glow linked
cargo test                       # full suite, ~25s
cargo test vault::store          # single module / group
cargo test create_open_roundtrip # single test by name
cargo fmt
cargo clippy

# Linux AppImage; needs the ALSA headers above. `release.yml` builds every
# platform and attaches the artifacts to a `vX.Y.Z` release.
scripts/appimage.sh

# macOS only: universal .app, .dmg and .zip (macOS's own lipo/codesign/hdiutil).
scripts/package-macos.sh

# Android: the Rust half, then the release APK — the one command
# (android/README.md has the details). Needs ANDROID_HOME and JAVA_HOME on a
# JDK 17-25; the wrapper is Gradle 9.3.1 and refuses anything newer.
scripts/android-apk.sh install

# The Linux release job locally, through act + Podman (README has the socket setup).
act workflow_dispatch -W .github/workflows/release.yml -j linux \
  -P ubuntu-22.04=catthehacker/ubuntu:act-22.04 \
  -P ubuntu-latest=catthehacker/ubuntu:act-latest

# Browser extension lives next to the crate: ../extension (see its README)
../extension/install.sh          # builds the host + writes the native-messaging manifest
```

`features::shell::screen::probe_rows` is a debug probe that asserts nothing, so it
is `#[ignore]`d and `cargo test` is green as it stands. Run it on purpose when the
question is what a frame painted:

```bash
cargo test -- --ignored probe_rows --nocapture
```

## Architecture

Feature-first: `src/features/<capability>/`, one directory per capability, each owning
its own state and its own drawing, keeping its pieces to itself (`model.rs`, `logic.rs`,
`ui.rs` or `ui/`). A new capability is a new feature, never a new layer: no `services/`,
`controllers/`, `utils/` catch-alls.

**Ownership rule.** `App` is the composition root, not an API: it holds the loop
(`run_ui`, `pump`/`pump_sync`), the navigation and the cross-cutting resources
(`icons`, `prefs`, `generate`). A feature owns the behavior that only touches its own
state. When behavior must reach another part, it takes what it needs as a parameter
(`&mut Shell`, `&mut Boxes`) instead of a back-pointer to `App`; only what genuinely
coordinates several features stays on `App`. Passing the dependency is what makes the
signature show what the method actually touches.

Dependencies point inward, never up: `vault/` is the platform-agnostic core (**no
egui**, no knowledge of features) meant to be shared on mobile; `shared/` holds only
what several features need and imports no feature. Neither reaches up into `features/`.

**Beside the box.** `prefs.json` (settings, sources, remotes, failed-unlock counters),
`recent.json` (the boxes the Unlock screen offers) and `biometric.json` (the sealed
keys) live in the app data dir, never inside a box. `store::data_dir` owns that path —
Android hands it over from `internal_data_path` — and one `pandora.lock` holds it
against a second instance, because prefs and the vault are single-writer resources.

None of them writes down what a box holds, and none of them names a box: a key in any
of those files is `box_key`, a **digest** of the box's canonical path (`crypto::digest`,
128 bits of BLAKE2b), so a mirror or a wait is found again without the file listing
where somebody keeps their boxes. A file from before that spells a path there is
re-keyed on the first read and written back. `recent.json` does keep the path — that
list is the way back into a box — but nothing else: what a row is called is the file's
name, never the box's (`Recent::label`). The working copies live in
`sources/<digest>/`, not `sources/<kind>/<owner>/<repo>/`, and the folder an older build
left there is moved over once.

**One pane per record.** The strip holds `Passwords` (`features/entries`, a login),
`Files` (`features/drive`, a note or a document), `Codes` (`features/codes`, a one-time
seed) and `Settings`. Each record pane owns its own list *and* its own editor, which is
what keeps `if kind ==` out of the drawing code. The drive is the one pane that holds
two types: `vault::model::Entry` is the tagged enum, so a note cannot carry a document
and a file cannot carry text. `Kind` is only what the screen and the CSV call a row.
A seed can also be read out of a QR, which lands the text in the same parser a pasted
one goes through: on a desktop, from the picture a screenshot leaves on the clipboard
or from a file; on a phone, from the photo picker or from the live full-screen scanner
(`codes::grab_code` / `scan_code` / `poll_scan`). The reading is ZXing's (`rxing`),
because the hard case is a photograph of a screen: it finds the corners *and* corrects
the modules the panel's own pixel grid smudges. It runs on its own thread, one frame at
a time, so the preview never waits for a read.

**Platform split.** `lib.rs` is the crate: `main.rs` is only the desktop binary, and
`android_main` starts the same app. `cfg(target_os = "android")` is the only switch, and
it stays at the edge — `vault/` never learns what a platform is (Android hands it a
directory through `store::set_data_dir`). Platform calls live behind `shared/android.rs`,
the Rust end of one Java class:
`android/app/src/main/java/dev/pandora/Pandora.java`. One JNI signature per method, so a
rename on either side is a rename on both.

What the phone does not carry, and what a change must not put back: egui's
bundled fonts (read from `/system/fonts`, `theme::apply`) and rxing's `image`
feature (which drags in `imageproc` and every format `image` knows). The
renderer is wgpu on the desktop and glow on the phone: `glow` is 1.2 MB of APK
smaller there, and the case that kept it out — the locked phone — was re-run on
a device and paints. `App::ui`'s swapchain knob is wgpu-only and compiled out
on Android; the numbers and the way back are in `android/README.md`.

The desktop defaults to Glow: eframe is built with both renderers and
`PANDORA_RENDERER=wgpu` falls back at runtime; `--no-default-features` builds
wgpu-only without linking Glow. On the measured NVIDIA machine Glow was ~95 MiB
RSS / 60 MiB PSS / 26 MiB VRAM warm against wgpu's ~171 / 92 / 215; the first
(cold) run is heavier because it compiles shaders. `App` keeps a runtime `wgpu`
flag: the swapchain-latency call is wgpu-only and must not run under Glow.

On Linux the desktop runs native Wayland when the session offers it: the
compositor owns the resize gesture there. winit 0.30 only emits `DroppedFile`
from X11, so a session that needs drag-and-drop sets `PANDORA_X11=1` to pick
the X11 backend instead (`src/main.rs`). The swapchain runs at one frame of
latency normally; a resize briefly raises it to two (`App::ui`) so the edge
keeps up without leaving the whole app a frame behind the pointer.

## GitHub mirror

A box can be mirrored to a GitHub repo; the local file is the source and the repo a
copy — and only ever a **private** repo, because the file names, their sizes and the
dates they change are readable by whoever the repo is. The picker lists a public one
and will not offer it, and `sync::put` asks GitHub for `private` before it writes, so
a repo made public after it was picked is refused on the next push.
`features/sync/github.rs` is the whole network surface: no git, just the Contents
API moving one opaque encrypted blob. The OAuth device flow mints a session (`repo
read:org offline_access`), and the client id is build-time (`PANDORA_GITHUB_CLIENT_ID`,
`.cargo/config.toml`, not the repo); without it sign-in is refused as an unconfigured
build. A token is a credential: it is `Zeroizing`, and the system credential store
keeps the refresh token by default (`Prefs::remember`); turning that off takes it back
out.

**The sign-in opens its own page.** The frame that lands the code copies it and hands
GitHub's address — `verification_uri_complete` when GitHub sends one, so the code is
already inside — to the platform's opener (`App::open_page` → `browse`), because a
browser landing on top of the app must never hide the one value the user may have to
carry. Nothing waits on that browser: the opener is spawned and its child reaped on a
thread of its own, and on Linux the chain is `xdg-open` → `gio open` → `$BROWSER`,
since no single one is guaranteed to be there. The card keeps the code and its own
**Open GitHub** button, so a window that was closed, covered or never came up is a
retry and not a dead end.

**Rotation and the repo.** A password change cannot reach what the repo already holds:
every blob it was ever sent stays in that repository's history, and each of those
copies still opens with the password being replaced. The master-password card says so
before the button is pressed, and offers the one thing the app can do — take the box
out of the repo (`github::erase`, a delete that names the blob) so the next push starts
the file again from nothing. The commits behind it are the user's to rewrite; nothing
in the Contents API does that, and no code here pretends otherwise.

Every call blocks, so each runs on a worker thread and lands in `App::pump_sync`: the
UI thread never waits on the network. A pull remembers the blob sha and a push names
it, so a version this box never read comes back `Conflict` rather than being
overwritten. A read is one Contents API call, except past 1 MB: there the response
carries the `sha` and no `content`, so the bytes come from the Git Blobs API under that
same sha (`github::get_at` → `blob_at`). The ceiling was always the JSON reply, never
the file, which is why `sync::MAX_FILE` is GitHub's own 100 MB blob limit and nothing
smaller. A **pull names the generation** this
box is on (`Vault::seq`), and `store::adopt` refuses a copy older than that with
`store::ROLLED_BACK`: a repo can be rewound to an old blob, an old blob is a valid blob,
and the sha this box holds says nothing about a rewind. The restore from the stash is
the one copy that passes no floor — going back is what it is for. A commit subject is
plaintext in the repo, read by whoever holds it, so it is one fixed line
(`Fingerprint::SUBJECT`) and
never a name, a count or which record moved: `Fingerprint` records only *that* a
version went out. Automatic pushes wait `PUSH_QUIET` after the last
edit and are paced against the hourly write budget (`Quota`); reads are paced by
`STALE`. A fetched copy goes through `store::adopt`, and the working copy it replaces
is stashed, so a fetch is never what loses an edit that had not reached the repo.

## Browser extension

A Chrome/Firefox WebExtension that autofills logins lives next to the crate, in
`../extension/` (own folder, not built by cargo). It never reads the vault and never
runs the crypto: it speaks native messaging to `pandora-native-host` (`src/bin/`), whose
whole behaviour is `src/host/`. The host opens the box through `vault/` and holds the
derived key; the extension is handed a site's entries, and one password only after the
user picks one — a listing or a match can never carry one (`serve::Candidate` has no
password field, by construction). `host/` is a driver like `app`: it composes `vault/`,
`bridge/` and the recents list, and draws nothing. What a browser can reach is
`vault.logins` and nothing else: `serve::entries`, `serve::matches` and `serve::reveal`,
plus the host's own create/update, never look at a note or a code.

**Matching** is `vault/matching.rs`, shared by `host/` and the passwords pane. `parse`
reduces any host or URL to a `Site` (canonical host + registrable domain via the Public
Suffix List, IDN folded to punycode). Each of an entry's URLs carries a `Rule`: `Domain`
(the eTLD+1, the default), `Host` (that exact host), `Prefix` (normalised URL prefix) or
`Never` (an exclusion, exact host, that drops the whole entry). `rank` returns the most
specific rule that accepts the page — and it takes a `Login`, so a note or a code cannot
claim a page even by mistake. The stored address is trusted and the page is not, so every
rule asks *does the entry accept this page*, never the reverse. Rules that carry a public
suffix (`github.io`, `co.uk`) have no registrable domain, so they can only match by
`Host`; that is what keeps tenants of a shared suffix apart.

`Request::Match` (`bridge/protocol.rs`) takes `url` (the whole address) and still accepts
the older `host`-only form.

**The bridge, and why the extension stops asking for a password.** When the app is
running it listens on a local channel — a Unix domain socket at
`$XDG_RUNTIME_DIR/pandora-bridge.sock`, a named pipe at
`\\.\pipe\pandora-bridge-<digest>` on Windows; `PANDORA_BRIDGE_SOCKET` overrides the
path both platforms derive their channel from — and the host relays to it instead of
opening the file — the app already holds the key for the box on screen, which is the
KeePassXC shape.
The browser names the caller: Chromium puts the extension origin in the host's `argv[1]`,
Firefox the add-on id in `argv[2]`, and the host passes it along as `caller` because the
app cannot see the browser's process table. A caller the user has not approved waits on a
modal, remembered in `Prefs::browsers`, and gives up after 30 s. Two requests never leave
the host — `list_boxes` and `unlock`, because opening a box is the window's job — and
`Prefs::unlock_from_browser`, off by default, is the only thing that lets a browser do it
at all. Reads are answered by `bridge::serve`, shared by both peers; writes change the
app's live session and mark it dirty, so **the app is the only writer** and there is no
mtime conflict to detect. The channel is protected by being user-private — the socket
by its mode and directory, the pipe by `PIPE_REJECT_REMOTE_CLIENTS` and the creator's
DACL — not by the modal: same-user malware can claim any caller id, exactly as it can
against KeePassXC.
With no app listening the host is on its own again, which is what `tests/native_host.rs`
pins down.

**Writing.** `Create`, `Update` and `Generate` are the only requests that change
anything. They are unlocked-only, they name fields rather than a path, and they write
through `store::save` *after* a `refresh()`, so an edit the app made while the panel was
open survives. Nothing is ever written without the user clicking the panel's own button —
the content script has no auto-save path, and a wrong match cannot write. `Generate` is
pure: a password rides straight back to the page and the host stores nothing. A
`Candidate` is the only shape a listing, a match or a save reply takes, so none of them
can carry a password; `CodeInfo` is the same bargain for a seed. Only `reveal`,
`generate` and `code` ever return a secret, and the first two are picked by id: `reveal`
for the login and `code` for the seed the user chose — both derived on the spot, and
neither repeating what the box stores.

A flatpak browser is sealed off from the repo and from the real data dir. For Zen,
`install.sh` drops a copy of the host and the manifest inside the sandbox prefix, grants
the Pandora data dir read-only, and names it for the host through `PANDORA_DATA_DIR`
(applied by `host::run`, so the crate's `store::data_dir` contract is untouched). Zen
patched Gecko to honour `xpinstall.signatures.required=false`, so the installer also
sideloads the unsigned xpi into the profile with that pref set.

## Fingerprint unlock

The device's reader never reaches the vault: what a finger unlocks is the platform's
keystore, and what the keystore holds is a box's **derived key** (see
`features/auth/biometric/`). Consequences worth keeping:

- The seal is per box path and is written down as ciphertext only, in `biometric.json`
  next to the prefs. `Prefs::biometric` says whether to make one at all; it is made the
  next time that box's password works.
- An unlock through it runs no Argon2 — `store::open_keyed` takes the key straight to
  the file. A rotated password means a new salt, so the old key decrypts nothing and the
  password is asked for again; nothing has to be invalidated by hand.
- It is as strong as the platform keystore and no stronger. Never let it stand in for
  the master password anywhere else.

## Vault format

A vault is one portable file. v4 is a fixed 4 KiB header, 1 MiB encrypted chunks and
an encrypted manifest at the end. Only v4 opens; an older container is refused.

- `CURRENT_VERSION` is 4. The header is `PANDORA4\n` followed by JSON (KDF, a random
  `box_id`, and the manifest's offset, length and nonce), padded to 4 KiB. The
  manifest's AEAD tag authenticates the header fields, so the KDF parameters cannot
  be changed under the ciphertext. Changing what the header covers means a new
  version, not an edit. Only v4 boxes open; an older container is refused.
- A file or note body lives in one or more chunks outside the manifest. Each chunk is
  XChaCha20-Poly1305 with its own random nonce; its AAD binds the box id, the record
  id and kind, the chunk index, the chunk count and the plaintext length, so a chunk
  cannot be moved to another record or position. The manifest carries the index and
  is the only thing decrypted on open: `File.data` and `Note.body` are read on demand
  (`Vault::read_file`, `note_text`, `copy_file_to`).
- A file picked or dropped on the desktop stays where it is as a pending path until the save chunks it; a phone picker copy is moved to the app's staging
  folder, chunked on save and then removed. Nothing is read into memory whole.
- Password rotation re-encrypts every chunk: the writer notices the KDF changed and
  reads the old records through the old reader before writing with the new key.
- The plaintext payload is **four collections**: `folders`, `logins`, `drive` and
  `otps` — plus `trash`, the records deleted but not erased. `trash` holds `Trashed`,
  the owned mirror of `Any` with the place each record held, so a restore puts a drive
  entry back where it was. Nothing blind to the kind reads it: not `records()`, not
  `any()`, not the CSV, not a `Fingerprint`. `Vault::take` is the delete and
  `Vault::remove` the permanent erase; only emptying the trash drops a file's bytes.
  `drive` holds `Entry`, a tagged enum of `note` or `file`, so a document is a
  type and not a field — a note cannot hold a password, a code cannot be handed to a
  login form, and a note cannot carry bytes. `Kind` is a projection of that for the
  screen and the interchange file, never a field.
- A file's bytes travel inside the box as chunks, never as a separate clear file: a
  phone's picker copy is staged, chunked on save and deleted. A document is read only
  when it is previewed or exported; `MAX_PREVIEW` (64 MiB) is a preview ceiling, not a
  storage limit.
- **Leaving the box is never silent.** Both ways out — the CSV of the whole box
  (`export_csv`) and one document's bytes (`export_file`) — go through a `Confirm` that
  says it lands unencrypted in Downloads, names the exact file, and offers
  `Use a neutral file name`: `pandora-export.csv`, or `pandora-<digest>.pdf` with the
  extension kept so the file still opens. A file called after the record tells the
  folder — and the backup service watching it — what the box was keeping.
- Operations blind to the kind — select, drag, move, delete, commit, export — go through
  `Vault::any` / `Vault::any_mut` (`model::Any`), which is why a click carries an id and
  never a collection.
- `Vault::seq` is the box's **generation**: `store::save` bumps it, and it is the only
  thing that tells a copy older than the one on screen apart from a newer one. It rides
  inside the payload, so it is ciphertext like the rest, and it reads as absent (0) in a
  box written before it existed.
- The file is not frozen yet, so the payload's shape may still change. Once it ships, a
  new persisted field needs `#[serde(default)]` + `skip_serializing_if`, and never
  renames or drops one. `url_match` on a login is the pattern: a vault nobody gave a
  rule to writes the JSON it always did, and
  `an_empty_box_writes_no_empty_collection` guards the rest.
- Writes are atomic (`.tmp` + `fsync` + rename). Chunks pad their last plaintext block
  to 4 KiB, and the whole file is rounded to a 4 KiB block with a 64 KiB floor
  (`PAD_FLOOR`), so a small box is never its own few hundred bytes and its size does
  not leak.
- Every write keeps the version it displaced in one `<box>.pandora.bak` beside the file:
  a sync, a save or a repo fetch is never the last word on a box. That copy is written
  through `private_file` (`store::stash` → `copy_private`) and not with `fs::copy`: a
  stash inherits the box's mode otherwise, and a box somebody once left at 0644 would
  put a second, world-readable copy of the same secret in the folder. Settings restores
  that copy through `store::adopt`, which verifies it with the key the box already
  holds and stashes the current file in the same slot, so a restore can be taken back.
  A **password change drops that copy** (`store::forget_stash`), because what it holds
  is sealed with the key the change just replaced. Rotation does not reach further than
  this machine: see the mirror section for the repo's own history.
- A keyfile is an optional second factor. Losing it loses the vault, so its path is
  never stored inside the vault — and the box does not say whether one was used
  either: `store::open` derives with the keyfile when one is handed over and then
  with the password alone, and the AEAD tag says which key the box owns. One derivation
  in the usual case, two for a box that has a keyfile. A keyfile box opened without it
  answers with the same `crypto::WRONG` a bad password does, which is why the unlock
  form offers the keyfile field every time.
- Argon2id (64 MiB / 3 / 4) takes seconds: derive on the worker thread, never the UI
  thread, and show a spinner.
- Autosave is debounced (900 ms) and skipped while a sync conflict is pending; a save
  also refuses when the file changed under it. External edits are detected by file mtime,
  and a different box on disk asks the user instead of replacing the vault.

## Secrets

The threat model and the reporting channel live in [`SECURITY.md`](SECURITY.md). It is
the long form of "what this does not protect against"; when a boundary moves, that
file moves with it.

The master password is used only for derivation. Keys and secret fields are `Zeroizing`.
Keep secret material out of logs, status text and `Debug` output — the record types
hand-write their `Debug` so a panic cannot print one. No error repeats what a caller
sent either: a request the bridge does not serve is answered with one fixed sentence,
because an unknown shape can carry a password and the caller's own words are the last
thing that should come back out of a log or a panel.

- The clipboard countdown keeps a BLAKE2 fingerprint of what it copied, not the text,
  and clears only while reading the clipboard back still matches: something the user
  copied afterwards stays. `Prefs::clip_clear = 0` never clears.
- `Prefs::lock_after` locks on system idle (and on the OS locking or suspending) after
  flushing every open box; `0` disables it.

## UI

- Two schemes, one palette. `theme::palette()` is the only way to a colour: there are
  no `const` colours left to reach for, because the scheme is chosen at runtime (`Prefs`
  → `theme::set` → `theme::apply`, the same shape as `shared::i18n` and its language). A
  new colour goes in `Palette` with a value for both, never in the drawing code: a
  hardcoded one is a scheme that is broken in one of the two and a test that cannot see
  it. `theme::ACCENT` is the exception — the brand green does not move, and a test keeps
  it in step with `scripts/build-logo.sh`.
- The light scheme is held to AA for `text` and `muted` on every surface, and the
  selected fill is held to AA behind `text` (`both_schemes_keep_their_text_readable`).
  `faint` is for hints and only has to stay lighter than `muted`.
- The scheme and the language are the app's, not a box's: they are read before one
  opens and outlive every one of them, so they live in `settings::ui::general` — the
  bar's sliders, a dialog reachable with nothing unlocked. The gear on the box's strip
  is the box's own page (`Tab::Settings`), and each is named for what it holds
  ("General settings" against "Box settings").
- No OS title bar: the shell paints the drag region, window buttons and resize edges.
  Invisible interactive regions must use the `Sense::CLICK`/`Sense::DRAG` consts, not
  `click()`/`drag()`, or they become dead tab stops.
- A hand-painted control owes `theme::focus` for its ring and `resp.widget_info` for
  the screen reader's name.
- egui honours `Visuals::interact_cursor` on its own `Button` and nowhere else, so
  every other control asks for its own cursor — through `theme::check`, `theme::combo`,
  `theme::slider` or the scroll wrappers, or with `on_hover_cursor` on a painted
  response. The hand for a click, the open hand for a drag, the closed one held.
- Keyboard-first behavior lives in `App::shortcuts` (`features/boxes/logic.rs`). Use
  `consume_key`, never `key_pressed` (a modal eats its own Escape); gate bare keys on
  `typing(ctx)`. Ctrl+Z undoes the last delete — the record is only gone for good when
  the trash is emptied (`vault/model.rs`, shown in Settings). Ctrl+K opens the whole-box
  search (`vault/search.rs`, the palette in `features/find/`): it matches names, users,
  addresses, notes, folder names, a note's body and a code's name, and never a password,
  a seed or a file's bytes. The palette is an overlay, not a `Screen`, and its field
  takes the keyboard in its own frame (`Find::needs_focus`).
- Below 620 px (`shared::ui::responsive::BREAKPOINT`) the layout collapses to one column.
  Phone sizing goes through `responsive` (`target`, `row`, `glyph`, `grow`) — 44/42/18
  below the breakpoint, 28/34/15 above — never a hardcoded number.
- A form page is one column of `theme::COLUMN`, centred; settings and the generator
  both hang it that way, and a window narrower than the card gives the column its own
  width. No form page splits into piles: settings was tried in two and it read worse
  than the scroll it saved. Inside a card the controls take the row:
  `theme::slider` asks for the widest value the slider can show, because egui lays that
  value box out *beside* the rail and a rail sized to the card would push it past the
  frame; the lock select sets `combo_width` to the row and truncates its label, since
  `ComboBox::width` is a floor and not a ceiling.
- A page taller than the window scrolls, and only such a page wears a bar today: the
  auth pages ask `theme::scroll_when_needed`, whose bar takes its strip and paints
  itself only once there is something to scroll — the one exception to the stable
  gutter `theme::scroll` keeps, because a bar down the side of a centred card promises
  a scroll a form usually cannot keep. The three of them (`auth::welcome`,
  `auth::unlock`, the sources picker) hang on `theme::pane` and on `auth::top` so the
  wordmark starts at one height (`the_auth_screens_share_one_top_anchor`), and the
  picker keeps its own bands, capped by `available_height`. A **dialog does not
  scroll**: one taller than `chrome::body` is clipped, so its content stays under a
  380x520 window or gets a band of its own.
- New user-facing text goes through `shared::i18n` (`t`, `tf`); the English and Spanish
  catalogs must share a key set, which a test enforces.
- A dialog asks for its width with `theme::width`; `chrome::GAP` is the room the card
  keeps off the window's edge. It rides the content that is asked for, never the
  modal's own area: that area is what the backdrop is painted through.
- Layout tests run egui headless and assert on painted shapes; draw through
  `shared::ui::shot`, never a hand-built `RawInput`.

## Assets

PNGs under `assets/**` are embedded at compile time, so changing an icon is never just
the image: regenerate it, then wire the loader (and for a brand, its enum + `ALL`).
`scripts/*.sh` generate assets, not part of the build. The README banner is not made
here: it is `../generate-banner/projects/pandora/`, which keeps its own copy of the icon
and of the app shot, and `generate.sh` there writes `assets/banner.png`. The accent
colour is duplicated in code, in `build-logo.sh` and in that project's `content.yaml`;
the test guards the first two.
