#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app=/private/tmp/GitJukeAlertsQualification.app
if test -e "$app"; then echo "Refusing to overwrite existing qualification app" >&2; exit 1; fi
mkdir -p "$app/Contents/MacOS"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>dev.hmrguez.gitjuke.alerts.qualification</string><key>CFBundleName</key><string>GitJuke Alerts Qualification</string><key>CFBundleDisplayName</key><string>GitJuke Alerts Qualification</string><key>CFBundleExecutable</key><string>alerts-qualification</string><key>CFBundleVersion</key><string>1</string><key>CFBundlePackageType</key><string>APPL</string><key>LSMinimumSystemVersion</key><string>15.0</string></dict></plist>
PLIST
/usr/bin/clang -fobjc-arc -fblocks -framework Cocoa -framework UserNotifications "$root/src-tauri/src/notifications.m" "$root/scripts/fixtures/notification-qualification.m" -o "$app/Contents/MacOS/alerts-qualification"
echo "$app"
# Build only. Launch explicitly for local qualification; never starts the normal app.
