#!/bin/bash
# Builds build/Drainlog.app: the SwiftUI app plus drainlogd, the background recorder that the
# app registers as a login agent (Contents/Library/LaunchAgents/com.drainlog.agent.plist).
# Ad hoc signed. Usage: scripts/bundle.sh
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> drainlogd, universal (aarch64 + x86_64)"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked --manifest-path core/Cargo.toml --bin drainlogd --target "$target"
done
mkdir -p build
lipo -create -output build/drainlogd \
  core/target/aarch64-apple-darwin/release/drainlogd \
  core/target/x86_64-apple-darwin/release/drainlogd

echo "==> Drainlog app, release"
universal=(-c release --package-path app --arch arm64 --arch x86_64)
if swift build "${universal[@]}"; then
  bin=$(swift build "${universal[@]}" --show-bin-path)
else
  # The multi-arch build goes through Xcode's build system; without it, ship arm64 only.
  echo "note: the universal Swift build failed, so the app is arm64 only" >&2
  swift build -c release --package-path app --arch arm64
  bin=$(swift build -c release --package-path app --arch arm64 --show-bin-path)
fi

echo "==> build/Drainlog.app"
app=build/Drainlog.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchAgents" "$app/Contents/Resources"
cp app/Bundle/AppIcon.icns "$app/Contents/Resources/"
cp app/Bundle/Info.plist "$app/Contents/Info.plist"
cp app/Bundle/com.drainlog.agent.plist "$app/Contents/Library/LaunchAgents/"
cp "$bin/Drainlog" build/drainlogd "$app/Contents/MacOS/"
plutil -lint "$app/Contents/Info.plist" "$app/Contents/Library/LaunchAgents/com.drainlog.agent.plist"
codesign --force --deep -s - "$app"
codesign --verify --deep --strict "$app"
lipo -info "$app/Contents/MacOS/Drainlog" "$app/Contents/MacOS/drainlogd"
echo "Built $app"
