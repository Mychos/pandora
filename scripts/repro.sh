#!/usr/bin/env bash
# Build the same tree twice, each time from nothing, and compare the two AppImages byte
# for byte.
#
#   scripts/repro.sh
#
# The point is not that cargo repeats itself; it is that a stranger who checks out the
# same commit and runs the same script gets the same file. This script proves the half it
# can: no state survives from one pass to the next — each run deletes its target
# directory first, the state a fresh clone would have.
#
# The other half is a property of the flags, which `appimage.sh` sets: the paths rustc
# would embed (the checkout, the cargo home, the target directory) are remapped to
# `/build/...`, so the absolute path a build happens under does not reach the file. The
# compiler is pinned by `rust-toolchain.toml` and the timestamps come from the commit,
# which leaves the source as the only variable.
#
# One line on stdout, and the exit status says whether it holds:
#
#   reproducible: yes <sha256>
#   reproducible: no (<first> != <second>)
#
# The release job runs this after the artifact it ships is built: the file it writes is
# attached to the release, so the claim travels with the binaries.
set -euo pipefail

cd "$(dirname "$0")/.."

export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}

# Reuse the appimagetool the first build already fetched: what is being compared is
# this tree's build, not two downloads of the same tool, and CI has no reason to
# spend the network twice.
if [ -z "${APPIMAGETOOL:-}" ]; then
  tool=$(ls target/appimage/tools/appimagetool-*.AppImage 2>/dev/null | head -1 || true)
  if [ -n "$tool" ]; then
    export APPIMAGETOOL="$tool"
  fi
fi

work=target/repro
rm -rf "$work"
mkdir -p "$work"
# The tool is fetched once and reused across passes: two downloads of the same file
# would make the test slower, not stronger.
tool="$work/appimagetool.AppImage"

pass() {
  local name=$1
  echo "repro: building pass $name (this takes as long as a release build)" >&2
  rm -rf "$work/build"
  mkdir -p "$work/build"
  if [ -x "$tool" ]; then
    APPIMAGETOOL="$tool"
    export APPIMAGETOOL
  fi
  CARGO_TARGET_DIR="$work/build/target" scripts/appimage.sh > "$work/$name.log" 2>&1
  if [ ! -x "$tool" ]; then
    local fetched
    fetched=$(find "$work/build/target/appimage/tools" -maxdepth 1 -name 'appimagetool-*.AppImage' -print -quit)
    cp "$fetched" "$tool"
  fi
  local image
  image=$(find "$work/build/target/appimage" -maxdepth 1 -type f -name 'Pandora-*.AppImage' -print -quit)
  if [ -z "$image" ]; then
    echo "repro: pass $name produced no AppImage — see $work/$name.log" >&2
    exit 1
  fi
  cp "$image" "$work/$name.AppImage"
  sha256sum "$work/$name.AppImage" | awk '{ print $1 }'
}

first=$(pass one)
second=$(pass two)

if [ "$first" = "$second" ]; then
  echo "reproducible: yes $first"
else
  echo "reproducible: no ($first != $second)"
  echo "repro: the two logs are $work/one.log and $work/two.log" >&2
  exit 1
fi
