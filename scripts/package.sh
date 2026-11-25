#!/usr/bin/env bash
# Build the Linux and Windows releases from Linux into dist/:
#   Qimist-<version>-linux-x86_64.tar.gz   (with install.sh)
#   Qimist-<version>-windows-x64.zip       (plugins + Qimist.exe)
#   Qimist-<version>-windows-setup.exe     (if makensis is installed: dnf install mingw32-nsis)
# Windows is cross-compiled: rustup target add x86_64-pc-windows-gnu, and the
# mingw64 gcc toolchain. macOS builds need a Mac: packaging/macos/build-pkg.sh
# (or the GitHub workflow builds all three).
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
offline=${CARGO_OFFLINE:+--offline}
mkdir -p dist

# ── Windows ──
win=dist/windows
rm -rf "$win" target/bundled && mkdir -p "$win"
cargo xtask bundle qimist --release $offline --target x86_64-pc-windows-gnu
cargo build --release $offline -p qimist --bin qimist-standalone --target x86_64-pc-windows-gnu
cp -r target/bundled/Qimist.vst3 target/bundled/Qimist.clap "$win/"
cp target/x86_64-pc-windows-gnu/release/qimist-standalone.exe "$win/"
cp docs/MANUAL.html "$win/"
cp LICENSE "$win/LICENSE.txt"
stage="dist/Qimist-$version-windows-x64"
rm -rf "$stage" && mkdir -p "$stage"
cp -r "$win/Qimist.vst3" "$win/Qimist.clap" "$win/MANUAL.html" "$win/LICENSE.txt" "$stage/"
cp "$win/qimist-standalone.exe" "$stage/Qimist.exe"
cat > "$stage/INSTALL.txt" <<TXT
Qimist $version for Windows (64-bit)

Easiest: run Qimist-$version-windows-setup.exe if you have it.

By hand:
  VST3:  copy the Qimist.vst3 folder to  C:\Program Files\Common Files\VST3
  CLAP:  copy Qimist.clap to             C:\Program Files\Common Files\CLAP
  App:   Qimist.exe runs on its own with your default mic and speakers.

Windows may warn that the app is from an unknown publisher (it isn't signed):
click "More info" -> "Run anyway". MANUAL.html explains everything else.
TXT
rm -f "dist/Qimist-$version-windows-x64.zip"
(cd dist && zip -qr "Qimist-$version-windows-x64.zip" "Qimist-$version-windows-x64")
rm -rf "$stage"
if command -v makensis >/dev/null; then
    makensis -V2 -DVERSION="$version" -DSRC="$PWD/$win" -DOUT="$PWD/dist/Qimist-$version-windows-setup.exe" packaging/windows/qimist.nsi
else
    echo "makensis not found: skipping the Windows installer (dnf install mingw32-nsis)"
fi

# ── Linux ── (last, so target/bundled ends up holding the Linux plugins)
rm -rf target/bundled
cargo xtask bundle qimist --release $offline
cargo build --release $offline -p qimist --bin qimist-standalone
stage="dist/Qimist-$version-linux-x86_64"
rm -rf "$stage" && mkdir -p "$stage"
cp -r target/bundled/Qimist.vst3 target/bundled/Qimist.clap target/release/qimist-standalone run-qimist.sh docs/MANUAL.html "$stage/"
cp packaging/linux/install.sh "$stage/"
cp LICENSE "$stage/LICENSE.txt"
tar -C dist -czf "dist/Qimist-$version-linux-x86_64.tar.gz" "Qimist-$version-linux-x86_64"
rm -rf "$stage"

echo
command ls -lh dist/*.zip dist/*.tar.gz dist/*.exe 2>/dev/null || true
