# Security policy

Pandora is an offline secrets vault: logins, notes, documents and one-time seeds
inside one encrypted file. This document says what that file is meant to protect,
what it is not, and how to report something that looks wrong.

**Pandora has not been audited.** Nothing here is a guarantee. The code is the
description of itself; this page is a map to it, not a promise on top of it.

## Reporting a vulnerability

Report privately: open a draft security advisory on the project's own repository
page (**Security → Advisories → New draft**; it is private until it is published),
or mail the maintainer named in the repository. Do not open a public issue, and do
not attach a real vault or a real password — a synthetic reproducer is enough, and
a box built with a throwaway master password is the right kind of evidence.

What helps: the version (`Pandora v0.1.0`, shown at the bottom of Settings), the
platform, the steps, and which of the assumptions below the finding breaks. Expect
an acknowledgement within a week. There is no bounty; there is a credit line if you
want one.

## What Pandora protects

A `.pandora` file is one portable artifact. The threat it is built against is the
file leaving your control — a backup, a USB stick, a laptop that gets stolen, a
private repo on somebody else's server — and remaining unreadable without the
master password.

- **At rest.** The manifest and every chunk are XChaCha20-Poly1305 with a random
  24-byte nonce, under a key derived with Argon2id (64 MiB, 3 passes, 4 lanes,
  16-byte salt). Header parameters are authenticated as the manifest's AEAD
  associated data, so they cannot be turned down under the ciphertext
  (`src/vault/crypto.rs`, `vault/store.rs`). Chunk AAD binds the box id, record id
  and kind, chunk index and plaintext length, so chunks cannot be moved.
- **The optional keyfile.** A second factor: the BLAKE2b-512 digest of the file is
  appended to the password before derivation, so the password alone is not enough
  and the file alone is not either. The box does not record whether a keyfile
  was used; the AEAD tag says which key the box owns. Losing the keyfile loses
  the vault.
- **Size.** Chunk plaintext is padded to a 4 KiB block and the whole file is
  rounded to a 4 KiB block with a 64 KiB floor, so a box with one login is not
  visibly smaller than a box with a hundred.
- **In memory.** The master password is used only for derivation and dropped; the
  derived key and every secret field are `Zeroizing`. Record types hand-write
  `Debug` so a panic cannot print one. Errors never repeat what a caller sent, and
  requests the bridge does not serve get one fixed sentence, because an unknown
  shape can carry a password.
- **The clipboard.** A copied secret is cleared after a countdown (30 s by
  default; `0` never clears). What is remembered is a BLAKE2 fingerprint of the
  text, not the text, and it is only cleared while reading the clipboard back
  still matches — something you copied afterwards is left alone.
- **Leaving the box.** The CSV of the whole box and one document's bytes are the
  only ways out, both go through a confirmation that says the file lands
  in plaintext, and both offer a neutral name (`pandora-export.csv`,
  `pandora-<digest>.pdf`) so the folder — and whatever watches it — does not learn
  what the box was keeping.

## Who is assumed honest

- **You**, and your master password. It is never stored, never logged and never
  sent anywhere; there is no recovery, by design.
- **The operating system**: its random number generator, its file permissions, its
  keystore, its clipboard. Pandora asks the OS and believes the answer.
- **The device's screen lock**, for the fingerprint path — and only there.

## What is explicitly out of scope

These are documented design boundaries, not unknown gaps:

- **Malware running as you.** An unlocked box is plaintext in process memory. A
  keylogger, a debugger attached to the process, a memory dump, root — any of them
  reads what you can read. Pandora does not try to defend against code that already
  runs with your rights.
- **The file's metadata.** The size is padded, but the name you gave it, its path,
  its mtime and how often it changes are the filesystem's, not the vault's. A repo
  keeps the dates its commits land on.
- **A mirror's history.** Rotation changes the key the file uses now; it does not
  rewrite what a repo already holds. Every blob ever pushed still opens with the
  password being replaced. The rotation card says so before the button is pressed
  and offers the one thing the app can do — take the box out of the repo — and the
  commits behind it are yours to rewrite, because no Contents API call does that.
- **The CSV export.** Plaintext by design, after a confirmation that names the file.
- **A browser's own trust.** An extension that is compromised, or a page that fools
  it, is outside what the vault can see.
- **Coercion and shoulder-surfing.** No duress mode, no plausible deniability.
- **iOS.** Not wired up; there is no phone build to attack.

## The browser bridge

The extension never reads the vault and never runs the crypto. It speaks
native messaging to `pandora-native-host`, which opens the box through `vault/`
and holds the derived key; the extension is handed a site's entries, and one
password only after the user picks one. The same holds for a one-time seed: the
panel is given its name, and the code comes back only for the seed the user chose,
derived on the spot. A listing or a match cannot carry a secret — `Candidate` and
`CodeInfo` have no field for one, by construction.

When the app is running it listens on a local socket
(`$XDG_RUNTIME_DIR/pandora-bridge.sock` on unix, a named pipe on Windows, either way
user-private) and the host relays to it
instead of opening the file, so the extension never needs the master password at
all. The browser names the caller, and a caller the user has not approved waits on
a modal for 30 s and is then refused.

The honest boundary: **the socket is protected by being user-private, not by the
modal.** Same-user malware can claim any caller id, exactly as it can against
KeePassXC. The approval modal stops a browser you did not mean to let in, not a
process that already runs as you.

`Prefs::unlock_from_browser`, off by default, is the only thing that lets the
extension open a box on its own. Turning it on means the master password can be
typed into a browser window; the setting says so.

## Fingerprint unlock

The reader never reaches the vault. A finger unlocks the platform keystore, and
what the keystore holds is a box's derived key, sealed per box path and written
down as ciphertext only (`biometric.json`, next to the prefs).

- An unlock through it runs no Argon2 — the key goes straight to the file.
- A rotated password means a new salt, so the old sealed key decrypts nothing and
  the password is asked for again. Nothing needs invalidating by hand.
- **It is as strong as the platform keystore and no stronger.** On a phone where
  the keystore is weak, so is this. It never stands in for the master password
  anywhere else.

## The things written beside the box

`prefs.json` (settings, sources, remotes, failed-unlock counters), `recent.json`
(the boxes the Unlock screen offers) and `biometric.json` (sealed keys) live in the
app data directory, never inside a box. None of them records what a box holds, and
none of them names a box: a key there is `box_key`, a 128-bit BLAKE2b digest of the
box's canonical path, so a mirror or a wait is found again without a file listing
where somebody keeps their boxes. `recent.json` does keep the path — that list is
the way back in — but what a row is called is the file's name, never the box's.

A `pandora.lock` in that directory holds the settings and the vault against a
second instance, because prefs and the vault are single-writer resources.

Beside a box there is `<box>.pandora.bak`: the one previous version, written
through `private_file` rather than `fs::copy`, so a box somebody once left at 0644
does not put a second, world-readable copy of the same secret in the folder. A
password change drops that copy, because it is sealed with the key the change just
replaced.

Unlock limits are counted per box file (failures, and until when the box is
refused); `0` attempts never refuses. An unlock that works forgets the failures.

## Network surface

- No account, no server, no telemetry, no update check.
- The only request the app can make on its own is the GitHub mirror, and only
  after the user adds a repo. It asks GitHub for `private` before every write, so a
  repo made public after it was picked is refused on the next push.
- The token is a credential: it is `Zeroizing`, and the system credential store
  keeps the refresh token only while the user leaves that on.
- A commit subject is plaintext in the repo, so it is one fixed line and never a
  name, a count or which record moved.

## What is not a vulnerability

- Reading an unlocked box's memory, as the same user.
- The CSV export containing plaintext — it is what export means, and the prompt
  says so.
- A repo's history still opening with an old password after a rotation; the app
  documents it and offers the delete.
- The extension seeing a site's entry names, usernames and URLs: that is what it
  is for, and no password rides on a listing.
- Refusing a box that is not v4, or a copy older than the one on screen
  (`store::ROLLED_BACK`) — that is the guard working.
- The user's own lost keyfile or forgotten password. There is no recovery path, and
  that is the design.

## Cryptography, in one table

| What | How | Where |
| --- | --- | --- |
| Key derivation | Argon2id, 64 MiB, 3 passes, 4 lanes, 16-byte salt | `vault/crypto.rs` |
| Keyfile | BLAKE2b-512 of the file, appended to the password before derivation | `vault/crypto.rs` |
| Encryption | XChaCha20-Poly1305, random 24-byte nonce per chunk and manifest | `vault/crypto.rs`, `vault/store.rs` |
| Header binding | header as the manifest's AEAD associated data (v4) | `vault/store.rs` |
| Clipboard fingerprint | BLAKE2, never the text | `vault/crypto.rs`, `features/shell/clipboard.rs` |
| Box key in prefs | 128-bit BLAKE2b digest of the canonical path | `vault/crypto.rs`, `crypto::digest` |
| Phone seal | platform keystore (Keystore/Keychain) over the derived key | `features/auth/biometric/` |

There is no Pandora-designed cipher, mode or construction. Every primitive is a
well-known one from the RustCrypto crates, chosen for what it is.

## Supported versions

The project is pre-1.0 and single-track: fixes land on `main`. This version only
reads and writes v4; an older container is refused rather than migrated. The
format is not frozen yet, so a new persisted field is always optional.
