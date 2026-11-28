#!/usr/bin/env bash
# Pack GiroCrab.app into a compressed disk image with an /Applications link
# (drag-to-install window) and code-sign the image itself.
#
# Usage: scripts/make-dmg.sh <path/to/GiroCrab.app> <out.dmg> [identity]
#   identity  "Developer ID Application: …" (notarize.sh) or "-" / empty for an
#             unsigned image (local tests only).
#
# Why a signed DMG instead of a ZIP: the image itself carries a Developer ID
# signature. Gatekeeper checks it on download, and Sparkle can fall back to it
# before extraction if the EdDSA key ever has to be replaced (a ZIP cannot be
# code-signed), see docs/auto-updates.md.
#
# The window opens with a background (scripts/make-dmg-background.swift), the
# app and the link side by side and the app icon as volume icon. Finder lays it
# out (AppleScript) and stores it in the image's .DS_Store, so the first run asks
# once for permission to control Finder (Privacy & Security › Automation).
set -euo pipefail

[ $# -ge 2 ] || { echo "usage: $0 <path/to/GiroCrab.app> <out.dmg> [identity]" >&2; exit 2; }
APP="$1"; OUT="$2"; IDENTITY="${3:-}"
[ -d "$APP" ] || { echo "error: no app at $APP" >&2; exit 1; }
ICON="$APP/Contents/Resources/AppIcon.icns"
[ -f "$ICON" ] || { echo "error: no app icon at $ICON" >&2; exit 1; }
VERSION="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")"
VOL="GiroCrab"
# Finder addresses the volume by name: another one with the same name (an older
# image still mounted) would get the layout instead.
[ ! -e "/Volumes/$VOL" ] || { echo "error: eject /Volumes/$VOL first" >&2; exit 1; }

WORK="$(mktemp -d)"; MNT=""
cleanup() {
  [ -z "$MNT" ] || hdiutil detach -quiet -force "$MNT" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

STAGE="$WORK/stage"
mkdir -p "$STAGE/.background"
ditto "$APP" "$STAGE/GiroCrab.app"
ln -s /Applications "$STAGE/Applications"
swift "$(dirname "$0")/make-dmg-background.swift" "$STAGE/.background/background.tiff" "$VERSION"

# Read-write image first: Finder writes the window layout into it.
RW="$WORK/rw.dmg"
hdiutil create -quiet -volname "$VOL" -srcfolder "$STAGE" -fs APFS -format UDRW "$RW"
MNT="$(hdiutil attach -readwrite -noverify -noautoopen "$RW" | awk -F '\t' '$NF ~ /^\/Volumes\// { print $NF }')"
[ -d "$MNT" ] || { echo "error: could not mount the read-write image" >&2; exit 1; }

# Geometry matches make-dmg-background.swift: 660x400 content below a 32 pt
# title bar, icon centres. Path bar and status bar stay as the user has them
# (switching them off here would switch them off in every Finder window).
# The window is closed and reopened before the icons are placed: hiding the
# toolbar moves the content, and icons placed before that end up lower.
osascript - "$VOL" <<'APPLESCRIPT'
on run argv
  tell application "Finder"
    tell disk (item 1 of argv)
      open
      set current view of container window to icon view
      set toolbar visible of container window to false
      set bounds of container window to {200, 120, 860, 552}
      close container window
      open
      set opts to icon view options of container window
      set arrangement of opts to not arranged
      set icon size of opts to 128
      set text size of opts to 13
      set background picture of opts to file ".background:background.tiff"
      set position of item "GiroCrab.app" to {165, 196}
      set position of item "Applications" to {495, 196}
      update without registering applications
      delay 1
      close container window
    end tell
  end tell
end run
APPLESCRIPT

for _ in $(seq 1 20); do [ -f "$MNT/.DS_Store" ] && break; sleep 0.5; done
[ -f "$MNT/.DS_Store" ] || { echo "error: Finder did not store the window layout" >&2; exit 1; }
# Volume icon only now: -srcfolder leaves .VolumeIcon.icns out, and Finder's
# layout pass removes it together with the custom-icon flag.
cp "$ICON" "$MNT/.VolumeIcon.icns"
SetFile -a C "$MNT"
rm -rf "$MNT/.fseventsd"
sync
for _ in 1 2 3 4 5; do hdiutil detach -quiet "$MNT" && break; sleep 1; done
[ ! -d "$MNT" ] || hdiutil detach -quiet -force "$MNT"
MNT=""

rm -f "$OUT"
hdiutil convert -quiet "$RW" -format ULFO -o "$OUT"

if [ -n "$IDENTITY" ] && [ "$IDENTITY" != "-" ]; then
  codesign --force --timestamp --sign "$IDENTITY" "$OUT"
  codesign --verify --strict -vv "$OUT"
fi
echo "==> Disk image: $OUT"
