#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <binary> <version> <output-directory>" >&2
  exit 64
fi

binary=$1
version=$2
output_directory=$3
root_directory=$(cd "$(dirname "$0")/.." && pwd)
appdir="$output_directory/AppDir"
output="$output_directory/pollip_${version}_Linux_x86_64.AppImage"

command -v appimagetool >/dev/null || {
  echo "appimagetool must be installed and on PATH" >&2
  exit 69
}
command -v rsvg-convert >/dev/null || {
  echo "rsvg-convert must be installed and on PATH" >&2
  exit 69
}

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m 0755 "$binary" "$appdir/usr/bin/pollip"
install -m 0644 "$root_directory/packaging/linux/pollip.desktop" "$appdir/pollip.desktop"
rsvg-convert "$root_directory/packaging/linux/pollip.svg" -o "$appdir/pollip.png" --width 256 --height 256
install -m 0644 "$appdir/pollip.png" "$appdir/usr/share/icons/hicolor/256x256/apps/pollip.png"
ln -sf pollip.png "$appdir/.DirIcon"

ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 appimagetool "$appdir" "$output"
