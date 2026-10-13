#!/usr/bin/env bash
# Regression checks for AppImage license inclusion and development update feeds.
# No Rust build or network access: inspect the staged AppDir at the tool boundary.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/target/release" "$TMP/dist" "$TMP/fonts/fonts/example"
printf '#!/bin/sh\nexit 0\n' >"$TMP/target/release/photocraft"
cp "$TMP/target/release/photocraft" "$TMP/target/release/photocraft-cli"
chmod +x "$TMP/target/release/photocraft" "$TMP/target/release/photocraft-cli"
printf 'Synthetic font license fixture\n' >"$TMP/fonts/fonts/example/OFL.txt"
cat >"$TMP/appimagetool" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
args=("$@")
appdir="${args[${#args[@]}-2]}"
out="${args[${#args[@]}-1]}"
test -s "$appdir/usr/share/doc/photocraft/LICENSE-MIT"
test -s "$appdir/usr/share/doc/photocraft/LICENSE-APACHE"
test -s "$appdir/usr/share/doc/photocraft/OFL-example.txt"
if [ -n "$EXPECTED_UPDATE" ]; then
  [ "$#" = 5 ]
  [ "$2" = -u ]
  [ "$3" = "$EXPECTED_UPDATE" ]
else
  [ "$#" = 3 ]
fi
[ "$1" = --no-appstream ]
: >"$out"
SH
chmod +x "$TMP/appimagetool"
export CARGO_TARGET_DIR="$TMP/target" DIST="$TMP/dist" APPIMAGETOOL="$TMP/appimagetool"
export CRAFT_FONTS_DIR="$TMP/fonts" PHOTOCRAFT_VERSION=0.0.0 PHOTOCRAFT_BUILD_DATE=2026-01-01
export GITHUB_REPOSITORY=example/photocraft
EXPECTED_UPDATE="gh-releases-zsync|example|photocraft|latest|photocraft-*-linux-$(uname -m).AppImage.zsync"
export EXPECTED_UPDATE
env -u PHOTOCRAFT_APPIMAGE_UPDATE_INFO bash "$HERE/package.sh" --skip-build --formats appimage
export EXPECTED_UPDATE='https://example.invalid/photocraft.zsync'
PHOTOCRAFT_APPIMAGE_UPDATE_INFO="$EXPECTED_UPDATE" bash "$HERE/package.sh" --skip-build --formats appimage
export EXPECTED_UPDATE=''
PHOTOCRAFT_APPIMAGE_UPDATE_INFO='' bash "$HERE/package.sh" --skip-build --formats appimage
echo 'AppImage licenses and update feed overrides ok'
