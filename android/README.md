# Pandora on Android

Two halves: Rust builds the app (`src/lib.rs` → `android_main`), Gradle packages
it. Nothing of the vault lives here — this directory is the platform's own side,
and the only Kotlin/Java is the three things no crate covers:

```
app/src/main/java/dev/pandora/MainActivity.java     the activity: loads the cdylib, draws edge to edge
app/src/main/java/dev/pandora/Pandora.java        keystore + prompt, file picker, clipboard, browser, GitHub token, the phone's own bar
app/src/main/java/dev/pandora/Autofill.java       the native half of autofill: one string per answer
app/src/main/java/dev/pandora/AutofillService.java the phone's fill panel: fields, datasets, codes, saves
```

`src/shared/android.rs` is the Rust end of `Pandora.java`: strings in, strings
out, one JNI signature per method. `src/shared/autofill/jni.rs` is the Rust end of
`Autofill.java`, in the other direction: Java asks, the copy of the open box
answers. Renaming a method in one means renaming it in the other.

## Build

One command, from the repository root:

```bash
ANDROID_HOME=$HOME/Android/Sdk JAVA_HOME=/path/to/jdk-21 scripts/android-apk.sh
ANDROID_HOME=$HOME/Android/Sdk JAVA_HOME=/path/to/jdk-21 scripts/android-apk.sh install
```

It compiles the `.so`, drops it in `jniLibs` and packages the **release** APK (or
installs it over `adb`). It needs `JAVA_HOME` on a JDK **17 to 25** — the wrapper
is Gradle 9.3.1, and on JDK 26+ Groovy dies with `Unsupported class file major
version 71`, which reads like a broken build file rather than a wrong JVM — and
`ANDROID_HOME`, which also locates the NDK.

The two steps by hand, which is all the script does:

```bash
rustup target add aarch64-linux-android

# The .so the activity loads (see `android.app.lib_name` in the manifest).
# Not `cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release`:
# against NDK 26 that dies with `unable to find library -laaudio`, so the linker
# comes from the NDK's own variables instead.
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<version>
env 'CC_aarch64-linux-android=aarch64-linux-android29-clang' \
    'CXX_aarch64-linux-android=aarch64-linux-android29-clang++' \
    'AR_aarch64-linux-android=llvm-ar' \
    'CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android29-clang' \
    PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH" \
    cargo build --target aarch64-linux-android --release
cp target/aarch64-linux-android/release/libpandora.so \
    android/app/src/main/jniLibs/arm64-v8a/

# Package and install.
ANDROID_HOME=$HOME/Android/Sdk ./android/gradlew -p android :app:installRelease
```

Or open `android/` in Android Studio and Run. Either way the Rust half is built
by hand first: Gradle only packages what is already in `jniLibs`.

`arm64-v8a` is the only ABI built, and the only one worth packaging: every phone
that can run minSdk 29 has it, and each extra ABI adds an 11.5 MB `.so` to the APK.
An `x86_64` build is an emulator's, not a phone's, so nothing packages it:
`jniLibs/` is git-ignored and `scripts/android-apk.sh` leaves only the `arm64-v8a`
one behind. Another one is the same
build with another `-t` in `cargo ndk` or another target triple by hand
(`armeabi-v7a` → `armv7-linux-androideabi`, `x86_64` → `x86_64-linux-android`),
each with its own `rustup target add` — and it builds today: nothing in either half
assumes arm64.

## The release keystore

Android installs an APK as an update only when it carries the same signing key as
the copy already there. That key is `android/pandora-release.jks`: one file, and
whoever holds it can sign an APK every existing user will accept as an update. It
is not in the repository (`.gitignore` keeps `android/*.jks` out), and it must
not be.

```bash
keytool -genkeypair -keystore android/pandora-release.jks -storetype PKCS12 \
    -alias pandora -keyalg RSA -keysize 4096 -validity 10000 \
    -dname "CN=Pandora, O=Pandora-Secrets, C=ES"
```

A release reads it from four repository secrets, and `release.yml` fails in
`meta` — before anything builds — when a tag run is missing one:

| Secret | What it is |
| --- | --- |
| `PANDORA_KEYSTORE_BASE64` | the whole file, `base64 -w0 android/pandora-release.jks` |
| `PANDORA_KEYSTORE_PASSWORD` | the store's password |
| `PANDORA_KEY_ALIAS` | `pandora` |
| `PANDORA_KEY_PASSWORD` | the same string: PKCS12 keeps no second password |

The `android` job decodes the file, proves the bytes and the password open
together with `keytool -list`, and only then exports the path to Gradle.

With none of them set, `app/build.gradle` falls back to the debug key, so a local
build — and `scripts/android-apk.sh` on a machine that has no keystore — still
installs over `adb`. That APK cannot update a released install, and a released
APK cannot take its place: the signatures differ, and the installer refuses.

`versionCode` is derived from `appVersion` (`major * 10000 + minor * 100 + patch`),
so the only number to bump is `appVersion`; `meta` refuses a name that is not
`MAJOR.MINOR.PATCH`.

The key in use has the certificate fingerprint
`F7:93:60:11:A9:78:DB:1F:8C:15:1A:AD:FA:9F:95:2B:43:29:D1:BB:8E:77:65:D3:FC:0C:7E:E5:E9:BA:6A:DD`
(SHA-256); `keytool -list -v` on a copy checks it still matches before that copy
signs anything.

Losing it is not recoverable, only survivable: no update can replace a copy signed
with a different key, so every user uninstalls the app and installs it again once.
Their boxes are separate files and that uninstall does not touch them. Back it up
somewhere that is not the build machine, keep it out of logs, and treat it as what
it is — the ability to hand every user a build of someone else's choosing.

## What the phone decides

- **minSdk 29** (Android 10): the oldest release the app claims, and the first it
  was run on. `androidx.biometric` covers the fingerprint from 28 up and the
  keystore key that demands user authentication exists from 24 on, so nothing
  needs a version guard. Below it the master password is the only door — which is
  the door anyway.
- **GameActivity**, not a plain Activity: it is the class `android-activity` and
  `eframe` expect, and it brings the input method connection the password field
  needs. Its `prefab` C++ glue stays **off** — the crate brings its own.
- **The window is edge to edge, the status bar is hidden, the navigation bar is
  not.** The app paints its own chrome and uses the whole screen, cutouts
  included, so the framework insets nothing and the app asks for the bar's height
  itself (`Pandora.footer` → `android::footer`). The navigation bar stays on show:
  it is the way out of the app, and the bar at the bottom leaves it its room.
- **R8 on, and three keep rules.** The release build shrinks the dex from 7.2 MB to
  0.6 MB, which is most of what appcompat, `androidx.fragment` and the Kotlin
  stdlib were carrying for three Java files. `android/app/proguard-rules.pro`
  keeps `dev.pandora.Pandora`'s methods whole, because `shared/android.rs` asks for
  them through JNI, one name at a time, and nothing in the dex references them; it
  keeps `dev.pandora.Autofill` for the mirror-image reason (the `Java_…_Autofill_*`
  symbols carry the class name) and `AutofillService` because the manifest names it.
  `MainActivity` is in the manifest, so the framework's own rules cover it.
- **glow, and not wgpu.** wgpu, naga and ash were the largest single lump of
  code in the crate: with them the APK weighed 7.58 MB, without them 6.39 MB, and
  6.29 MB after the rest of the size work. The choice was settled on the device,
  not on that number: on a Mi 9T (Adreno 618)
  the glow build paints the unlock screen, survives three trips to the background
  and back, a rotation, and — the case that had never been reproducible from
  `adb` — the screen off, the phone locked and the keyguard dismissed. `logcat`
  stays clean and the fonts still come from `/system/fonts`. `App::ui`'s
  swapchain knob (`eframe::SurfaceConfig`) is a wgpu-only API, so it and the
  wgpu `on_exit` signature are compiled out on Android. Going back means putting
  `"wgpu"` in the Android `eframe` features, dropping `"glow"`, and restoring
  those two.
- **The fonts come from the phone.** egui's bundled faces are ~1 MB of `.rodata`
  and every Android carries the same ones in `/system/fonts`, so the phone build
  turns `default_fonts` off and `theme::apply` reads Roboto, a monospace and a
  symbols face — the last one because the UI draws `✎`, `→` and `⇄` as text and
  neither of the other two has those glyphs, and because the name that face goes
  by changes with the release (`NotoSansSymbols2-Regular.ttf` from Android 11,
  `NotoSansSymbols-Regular-Subsetted.ttf` on 10 — and the `Subsetted2` file is a
  124-glyph sliver with none of them). `logcat` says which one it took.
- **The `.so` travels compressed** (`packaging { jniLibs { useLegacyPackaging } }`):
  it deflates to 46% of its size, so the download is ~5.3 MB instead of 11.5 MB.
  The installer copies it out on the device, which is the trade.
- **`androidx.browser`.** The GitHub sign-in opens in a custom tab — the phone's
  own browser, so the page comes up with whatever session is already there, and
  the link never lands in the GitHub app's web view. Without a custom-tab
  browser it falls back to a plain `ACTION_VIEW`.
- **No `dirs`.** The activity hands the app its own directory, which
  `vault::store::set_data_dir` takes before anything else runs.

## Autofill

Settings → Autofill, in the phone's own settings app, then pick Pandora. That is
the whole installation: there is no permission to grant and no toggle inside
Android's keyboard. The switch inside the app (Settings → Autofill) turns the copy
off; turn it back on and the panel returns at the next fill request.

What it does, and what it does not:

- **Logins on a WebView.** The service reads `ViewNode.getWebDomain()` and asks the
  app's copy which entries claim it. A native app has no URL, so nothing claims it:
  no panel there. This is the largest gap and it is deliberate — entries are keyed
  by URL, and inventing a key from a package name would fill forms nobody asked.
- **Codes anywhere.** A field whose hint or id says `otp`, `one-time`, `totp`,
  `2fa` or `token` gets one row per seed, with the code it holds now and the
  seconds left. A field named only `code` has to be numeric too. The list is not
  filtered by site: `Otp` has no URL (`src/vault/model.rs`).
- **Saving** asks the app to queue the login and lands it there, through the same
  `serve::create_login` the browser panel uses; an entry with the same name,
  username and password is skipped, the rule an import uses. Without a web domain
  there is nowhere to save it to, and with no box open the panel only offers
the way to open the app.
- **No box, no data.** The process may start on a fill request; if the app was never
  open there is no copy to read, and the only row is "Open Pandora".

## Fingerprint unlock, briefly
The app never sees a fingerprint. After a box is opened with its password, its
**derived key** is handed to `Pandora.seal`, which encrypts it under a keystore
key that only a live fingerprint can use (`setUserAuthenticationRequired(true)`,
`setInvalidatedByBiometricEnrollment(true)`). What the app writes down is the
ciphertext, one entry per box, in `biometric.json`.

Open again with a finger and `Pandora.unseal` shows the prompt, unseals the key,
and the vault opens **without running Argon2** — the derived key is the expensive
part, and it is already in hand. A box whose password was rotated has a new salt,
so the old key no longer decrypts it: the AEAD tag refuses it and the password is
asked for again. Enrolling a new fingerprint drops the keystore key the same way.

The seal is exactly as strong as the device's keystore, and no stronger: a rooted
phone that can use that key can open the box. It is a convenience over the master
password, never a replacement for it.

## Not wired to a picker yet

CSV import still answers "not on the phone yet": a phone's picker belongs to the
activity, not to a window. The picker itself is wired for three callers: the box
file (`src/features/auth/pick.rs`, which keeps the copy — it is the file that gets
unlocked and saved), a QR photo (`codes::scan_code`) and a document
(`drive::pick_files`). The last two read the copy the activity left in the app's own
storage and then delete it: the plaintext copy is not the user's file. A keyfile is
typed rather than picked for the same reason.

## Not verified

No test here touches any of this; it needs a phone. What has been run:

- **Nothing of the autofill service.** It compiles (`javac` against
  `android-35`, the JNI half with `cargo ndk check`), the manifest links with
  `aapt2`, and the `-keep` rules are in place, but no fill request has ever been
  served on a device. The checklist below is the one to run first.

### What ran on an emulator (0.1.1, x86_64, API 35)

Confirmed on a Pixel 7 AVD, with the debug APK and a throwaway WebView app loading
`http://localhost:8000` through `adb reverse`:

- The service is bound and labelled **Pandora** in `dumpsys autofill` once
  `autofill_service` holds the flattened component (`dev.pandora/.AutofillService`);
  the dotted form alone resolves to no service and nothing is ever asked.
- **The lib loads in a process with no activity.** `nativeInit` ran, cached the VM
  and the context, and read `getFilesDir()` through them: `logcat -s pandora` said
  `autofill: ready, files at /data/user/0/dev.pandora/files`.
- **A fill request reaches the service and its response is drawn**: with the box
  open, a WebView form got one row — name, username and the stored URL that claimed
  the page — and picking it filled both fields. With the box locked, the only row
  was **Open Pandora**.
- **A bug the plan could not have caught**: a dataset carrying only
  `setAuthentication` is refused by `Dataset.Builder.build()` (*"at least one of
  field or datatype must be set"*), which killed the service process mid-request.
  `open()` now sets an empty value on every field it was offered for; the auth
  intent answers cancelled, so nothing is ever written.

What the emulator also showed, and is not our bug:

- **Chrome does not participate.** Focusing a field on an http page produced no
  fill request at all — Chrome keeps web forms out of the Autofill Framework.
- **Some system screens opt out.** The Wi-Fi password field in Settings asks
  nobody either (`importantForAutofill` is off there).

### Autofill checklist

With the APK installed over an existing one (no reinstall, versionCode 101) and
one box open in the app:

1. **Not selected**: the phone's autofill picker offers only the system's own
   suggestions. Select Pandora in Settings → Autofill, then reopen the form.
2. **Login in a WebView**: the panel lists the entries claiming the page, best
   first, each with the stored URL that claimed it. Picking one fills both fields
   and no password is ever drawn.
3. **No entry claims it**: an entry with no URL, or a page nobody claims, gets no
   panel at all — not an empty one.
4. **Code**: a form asking for a one-time code gets one row per seed, with the
   seconds left. The code that lands is the one shown.
5. **Save**: submitting a form the framework offers to save lands a login in the
   app; submitting it twice does not land a second one; a changed password does.
6. **Locked**: lock the box (button, or `lock_after`), then trigger the panel. It
   offers only "Open Pandora" and the log says `no box open`. The same after the
   screen has been off and on.
7. **Switch off**: Settings → Autofill off, then trigger the panel again. Same
   answer as 6. Turn it back on: the panel returns once the box is open again.
8. **No crash on a cold start**: force-stop the app, then trigger the panel from
   another app. The process starts, `logcat -s pandora` shows `autofill: ready,
   files at …`, and the row says "Open Pandora".

- The Rust half compiles and links for `aarch64-linux-android`, and the APK
  assembles (manifest, resources, a JDK 17-25, R8 with the keep rule) at 6.3 MB:
  5.3 MB of it the compressed `.so`, 0.65 MB the dex. It also builds for
  `x86_64-linux-android` (19 MB `.so`), so nothing in the Rust half assumes arm64.
- **The release APK runs on a phone on glow** (Xiaomi Mi 9T, Android 10, Adreno
  618), and comes back from the background — window visible, surface and layer
  both there, seen from `dumpsys` on the way out and on the way back in, three
  cycles, a rotation, and the screen off → keyguard → unlock cycle that no `adb`
  had been able to reproduce before. The pixels were checked with a throwaway
  build that had `FLAG_SECURE` off (screenshot: the unlock screen, the logo, the
  recents list, in portrait and in landscape); the shipped build has the flag on
  and `screencap` returns 0 bytes. The fonts come off
  `/system/fonts` (Roboto, DroidSansMono and
  `NotoSansSymbols-Regular-Subsetted.ttf`, the Android 10 name for the one that
  carries `✎ → ⇄`). Nothing complains through JNI, so R8 kept
  `attach`/`resumed`/`footer` — a renamed method would have thrown at startup.
  Nothing panics, and `FLAG_SECURE` is on the window.
- The `.so` itself carries no console: `logcat -s pandora` is the whole
  diagnosis, and it only works because `android_main` sets the level explicitly
  (`with_max_level`) — `init_once` with a default config never calls
  `log::set_max_level`, and `log` starts at `Off`, so a panic would leave a
  process that vanished with nothing in the log. `Info` and not `Debug`: a
  launch at `Debug` is ~2500 lines, most of them `naga` narrating its own shader
  lowering, and they bury the three lines that matter; anything above `Info` —
  panics included — survives the change, and a renderer hunt is a one-word edit
  away.
- The live QR scanner runs on a device (Android 10, arm64): the camera opens and
  streams, and a code lands in the box — `Pandora.startCamera` → frames →
  `codes::poll_scan` → ZXing. The step that took finding was the reader, not the
  camera: a photograph of a screen smudges the modules under the panel's own pixel
  grid, and the fragile decoder found the corners yet read none of the bits.
- On an **x86_64 emulator** the software renderer is what decides whether it is
  usable, and by default it is not: with `-gpu swiftshader_indirect` the app
  starts and the emulator's own SwiftShader dies while compiling shaders, which
  is the emulator and not the app. That run was made while the phone still drew
  through wgpu (a Vulkan device there); on glow it has not been re-checked.
  Handing it the host's GPU (`-gpu host`, or `hw.gpu.enabled = yes` +
  `hw.gpu.mode = host` in the AVD's `config.ini`) is what works: the guest's GLES
  is the emulator's translator over the host driver. `adb exec-out screencap` over it comes back
  black — the window is `FLAG_SECURE` — so a look at the screen needs
  `adb emu screenrecord screenshot <dir>`, which reads the emulator's own
  framebuffer. An emulator still cannot stand in for a phone anywhere the
  keystore, the camera or the keyguard is the subject; it is only good for the
  drawing.

Still to check on a device:

- The phone's own back: the button and the back gesture walk the app one step and
  leave it from the root (`App::back_or_leave`), and a focused field eats the first
  press to put the keyboard away.
- That `FLAG_SECURE` really covers the recents thumbnail and screenshots.
- The prompt and the keystore seal (`Pandora.seal` / `unseal`), including the
  fallback after enrolling a second fingerprint.
- The file picker: import a `.pandora` from Files and unlock it.
- The GitHub sign-in: the browser opens on the device page, and "Keep me signed
  in" survives a restart (`Pandora.keep` / `recall`, a keystore key that asks for
  no finger).
- Auto-lock follows Pandora activity while it is foregrounded. Switching apps does
  not lock it; turning the screen off does. Check the screen-off receiver on-device.

`adb logcat -s pandora` gets everything the app says, panics included. A panic on
a phone is otherwise a process that vanished.
