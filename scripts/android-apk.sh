#!/usr/bin/env bash
# Build the Android half and package the APK. The Rust crate is compiled for
# aarch64 by hand (Gradle only packages what is already in `jniLibs`), which is
# what `android/README.md` documents; this is that, with the environment the
# toolchain needs spelled out:
#
#   scripts/android-apk.sh            # build the APK
#   scripts/android-apk.sh install    # build, then `adb install -r`
#
# Those are the *release* tasks: R8, shrunk resources and the debug key as the
# signer (see `android/app/build.gradle`), which is the APK whose size matters.
# `:app:assembleDebug` stays the one to reach for while working on the Java half.
#
# Needs `ANDROID_HOME` (SDK, for the NDK and Gradle) and `JAVA_HOME` on JDK 17.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ndk="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
: "${JAVA_HOME:?set JAVA_HOME to a JDK Gradle 9.3 runs on — 17 to 25, and 21 is what this was built with}"

major="$("$JAVA_HOME/bin/java" -version 2>&1 | head -1 | sed -E 's/.*"([0-9]+).*/\1/')"
# Gradle 9.3.1 itself sets the ceiling: on a JDK 26+ Groovy dies with
# "Unsupported class file major version", which reads like a broken build file
# rather than a wrong JVM. The old floor of exactly 17 is long gone with it.
if [ "$major" -lt 17 ] || [ "$major" -gt 25 ]; then
  echo "JAVA_HOME is JDK $major; Gradle 9.3.1 wants 17-25" >&2
  exit 1
fi

# minSdk 29 (see `android/app/build.gradle`): the API level the platform
# libraries are linked against.
api=29
target=aarch64-linux-android
toolchain="$ndk/toolchains/llvm/prebuilt/linux-x86_64"
# `cargo ndk` misses `-laaudio` against this NDK layout, so the linker comes from
# the NDK's own variables — the fallback `android/README.md` already lists. `env`
# rather than `export`: a variable name with a dash is not shell-legal, and only
# `env` passes it through.
cd "$root"
env "CC_$target=$target$api-clang" \
    "CXX_$target=$target$api-clang++" \
    "AR_$target=llvm-ar" \
    "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$target$api-clang" \
    "PATH=$toolchain/bin:$PATH" \
    cargo build --target "$target" --release --locked
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp "target/$target/release/libpandora.so" android/app/src/main/jniLibs/arm64-v8a/

task=:app:assembleRelease
[ "${1:-}" = install ] && task=:app:installRelease
ANDROID_HOME="$ANDROID_HOME" ./android/gradlew -p android "$task"

apk=android/app/build/outputs/apk/release/app-release.apk
ls -lh "$apk"
unzip -v "$apk" | grep -E "libpandora|classes.*dex|resources.arsc|Length" | head
