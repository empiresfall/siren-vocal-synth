#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

INSTALL=0
if [[ "${1:-}" == "--install" ]]; then
  INSTALL=1
fi

echo "==> Building SIREN Vocal Synth (chimera-vst, profile=release-vst)..."
cargo build --profile release-vst -p chimera-vst

DYLIB="$ROOT/target/release-vst/libchimera_vst.dylib"
if [[ ! -f "$DYLIB" ]]; then
  echo "ERROR: Expected built dylib at $DYLIB" >&2
  exit 1
fi

OUT_DIR="$ROOT/dist-vst"
VST3_BUNDLE="$OUT_DIR/Siren.vst3"
CLAP_BUNDLE="$OUT_DIR/Siren.clap"

rm -rf "$VST3_BUNDLE" "$CLAP_BUNDLE"
mkdir -p "$VST3_BUNDLE/Contents/MacOS" "$CLAP_BUNDLE/Contents/MacOS"

write_bundle() {
  local bundle_path="$1"
  local bundle_id="$2"

  cp "$DYLIB" "$bundle_path/Contents/MacOS/Siren"
  chmod +x "$bundle_path/Contents/MacOS/Siren"
  printf 'BNDL????' > "$bundle_path/Contents/PkgInfo"

  cat > "$bundle_path/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>English</string>
  <key>CFBundleDisplayName</key>
  <string>SIREN Vocal Synth</string>
  <key>CFBundleExecutable</key>
  <string>Siren</string>
  <key>CFBundleIdentifier</key>
  <string>${bundle_id}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>SIREN Vocal Synth</string>
  <key>CFBundlePackageType</key>
  <string>BNDL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CSResourcesFileMapped</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

  xattr -cr "$bundle_path" 2>/dev/null || true
  codesign --force --deep --sign - "$bundle_path"
  codesign --verify --deep --strict "$bundle_path"
}

write_bundle "$VST3_BUNDLE" "audio.chimera.siren.vst3"
write_bundle "$CLAP_BUNDLE" "audio.chimera.siren.clap"

echo "==> Built & signed bundles:"
echo "    VST3: $VST3_BUNDLE"
echo "    CLAP: $CLAP_BUNDLE"

if [[ "$INSTALL" -eq 1 ]]; then
  SYS_VST3="/Library/Audio/Plug-Ins/VST3"
  USER_VST3="$HOME/Library/Audio/Plug-Ins/VST3"
  if [[ -d "$SYS_VST3" && -w "$SYS_VST3" ]]; then
    DEST_VST3="$SYS_VST3/Siren.vst3"
  else
    mkdir -p "$USER_VST3"
    DEST_VST3="$USER_VST3/Siren.vst3"
  fi

  rm -rf "$DEST_VST3"
  cp -R "$VST3_BUNDLE" "$DEST_VST3"
  xattr -cr "$DEST_VST3" 2>/dev/null || true
  codesign --force --deep --sign - "$DEST_VST3"
  echo "==> Installed Ableton Live VST3 plugin to: $DEST_VST3"
fi
