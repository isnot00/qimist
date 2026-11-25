#!/usr/bin/env bash
# macOS release (run on a Mac, or by the GitHub workflow): universal VST3 +
# CLAP + Qimist.app, packed into one .pkg installer and a .zip.
# The result is not signed by Apple: first launch needs right-click → Open
# (or System Settings → Privacy & Security → Open Anyway).
set -euo pipefail
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=dist/macos
rm -rf "$out" && mkdir -p "$out/root-vst3" "$out/root-clap" "$out/root-app"

rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo xtask bundle-universal qimist --release
cargo build --release -p qimist --bin qimist-standalone --target x86_64-apple-darwin
cargo build --release -p qimist --bin qimist-standalone --target aarch64-apple-darwin

# Qimist.app around the universal standalone binary.
app="$out/root-app/Qimist.app"
mkdir -p "$app/Contents/MacOS"
lipo -create -output "$app/Contents/MacOS/Qimist" \
    target/x86_64-apple-darwin/release/qimist-standalone target/aarch64-apple-darwin/release/qimist-standalone
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

cp -R target/bundled/Qimist.vst3 "$out/root-vst3/"
cp -R target/bundled/Qimist.clap "$out/root-clap/"
# Ad-hoc signatures (no Apple account needed; Apple Silicon requires at least this).
codesign --force --deep -s - "$app" "$out/root-vst3/Qimist.vst3" "$out/root-clap/Qimist.clap"

pkgbuild --root "$out/root-vst3" --install-location /Library/Audio/Plug-Ins/VST3 --identifier com.qimist.vst3 --version "$version" "$out/vst3.pkg"
pkgbuild --root "$out/root-clap" --install-location /Library/Audio/Plug-Ins/CLAP --identifier com.qimist.clap --version "$version" "$out/clap.pkg"
pkgbuild --root "$out/root-app" --install-location /Applications --identifier com.qimist.app --version "$version" "$out/app.pkg"
productbuild --package "$out/app.pkg" --package "$out/vst3.pkg" --package "$out/clap.pkg" "dist/Qimist-$version-macos.pkg"

# Plain zip too, for people who'd rather copy the files themselves.
stage="$out/Qimist-$version-macos"
mkdir -p "$stage"
cp -R "$app" "$out/root-vst3/Qimist.vst3" "$out/root-clap/Qimist.clap" docs/MANUAL.html "$stage/"
cp LICENSE "$stage/LICENSE.txt"
(cd "$out" && ditto -c -k --keepParent "Qimist-$version-macos" "../Qimist-$version-macos.zip")
echo "dist/Qimist-$version-macos.pkg and .zip ready"
