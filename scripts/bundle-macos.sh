#!/usr/bin/env bash
# bundle-macos.sh — Create a macOS .app bundle for OpenClaw Node Widget
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

BINARY="${REPO_DIR}/target/release/openclaw-node-widget-rs"
VERSION=$(grep '^version = ' "$REPO_DIR/Cargo.toml" | head -1 | sed 's/.*= "//;s/"//')
APP_NAME="OpenClaw Node Widget"
APP_DIR="${REPO_DIR}/${APP_NAME}.app"

echo "==> Bundling ${APP_NAME} v${VERSION}"

# Clean previous bundle
rm -rf "$APP_DIR"

# Create .app structure
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
cp "$BINARY" "$APP_DIR/Contents/MacOS/openclaw-node-widget-rs"
chmod +x "$APP_DIR/Contents/MacOS/openclaw-node-widget-rs"

# Generate Info.plist with actual version
sed "s/VERSION_PLACEHOLDER/${VERSION}/g" "$REPO_DIR/macos/Info.plist" > "$APP_DIR/Contents/Info.plist"

# Convert icon_online.png to .icns
ICON_SRC="${REPO_DIR}/assets/icon_online.png"
ICONSET_DIR=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET_DIR"

for SIZE in 16 32 128 256 512; do
    sips -z $SIZE $SIZE "$ICON_SRC" --out "$ICONSET_DIR/icon_${SIZE}x${SIZE}.png" >/dev/null 2>&1
    DOUBLE=$((SIZE * 2))
    if [ $DOUBLE -le 1024 ]; then
        sips -z $DOUBLE $DOUBLE "$ICON_SRC" --out "$ICONSET_DIR/icon_${SIZE}x${SIZE}@2x.png" >/dev/null 2>&1
    fi
done

iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/icon.icns"
echo "==> Created icon.icns"

echo "==> Bundle created: ${APP_DIR}"
