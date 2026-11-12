#!/usr/bin/env bash
# Builds target/ios/Horizon.ipa (arm64, Metal). Signed ad hoc (`codesign -s -`) so it installs
# on a jailbroken device or in a simulator-style sideload; a real device wants your own signing
# identity. See IOS.md.
set -euo pipefail
cd "$(dirname "$0")/../.."

TARGET="${HORIZON_IOS_TARGET:-aarch64-apple-ios}"
PROFILE="${PROFILE:-release}"
# Controllers (gilrs) are the desktop's: the phones use Horizon's own touch controls.
FEATURES="${FEATURES:-audio}"
# The release this IPA is, as the APK build reads the manifest: the game names it in its log
# and reports (horizon-core release.rs) instead of the crate's version.
export HORIZON_VERSION="${HORIZON_VERSION:-$(sed -n 's/.*android:versionName="\([^"]*\)".*/\1/p' packaging/android/AndroidManifest.xml)}"
OUT=target/ios
APP="$OUT/Payload/Horizon.app"

echo "[+] Building horizon-client for $TARGET ($PROFILE, features: $FEATURES)"
cargo build --profile "$PROFILE" --target "$TARGET" -p horizon-client --bin horizon-client \
  --no-default-features --features "$FEATURES"

rm -rf "$OUT/Payload"
mkdir -p "$APP"
cp "target/$TARGET/$PROFILE/horizon-client" "$APP/Horizon"
chmod +x "$APP/Horizon"
# The game looks for `resources` beside its executable (game/options.rs), which is the bundle.
if [ -d ".local/horizon-client/26.3" ]; then
  cp -R .local/horizon-client/26.3 "$APP/resources"
else
  echo "warning: .local/horizon-client/26.3 is missing: run packaging/ios/fetch_assets.py first," \
       "or the app starts without Minecraft's files" >&2
fi
sed "s/__VERSION__/$HORIZON_VERSION/g" packaging/ios/Info.plist > "$APP/Info.plist"
printf 'APPL????' > "$APP/PkgInfo"

# An ad hoc signature: no team, no profile. Enough for a jailbroken device and for `simctl`
# style sideloads; Xcode's own signing replaces it for a normal developer account.
codesign --force --sign - --timestamp=none "$APP" 2>/dev/null ||
  echo "warning: codesign failed (no signing identity on this host); the IPA is unsigned" >&2

(cd "$OUT" && rm -f Horizon.ipa && zip -qry Horizon.ipa Payload)
echo "[+] $OUT/Horizon.ipa ($(du -h "$OUT/Horizon.ipa" | cut -f1))"
