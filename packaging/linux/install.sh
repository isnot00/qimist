#!/usr/bin/env bash
# Install Qimist for the current user: plugins into ~/.vst3 and ~/.clap, the
# standalone app into ~/.local/share/qimist with a menu entry and a `qimist`
# command. Run ./install.sh --uninstall to remove it again.
set -euo pipefail
cd "$(dirname "$0")"

share="$HOME/.local/share/qimist"
apps="$HOME/.local/share/applications"
bin="$HOME/.local/bin"

if [[ ${1:-} == --uninstall ]]; then
    rm -rf "$HOME/.vst3/Qimist.vst3" "$HOME/.clap/Qimist.clap" "$share" "$apps/qimist.desktop" "$bin/qimist"
    echo "Qimist removed (your presets in ~/.config/Qimist and exports in ~/Music/Qimist are kept)."
    exit 0
fi

mkdir -p "$HOME/.vst3" "$HOME/.clap" "$share" "$apps" "$bin"
rm -rf "$HOME/.vst3/Qimist.vst3"
cp -r Qimist.vst3 "$HOME/.vst3/"
cp Qimist.clap "$HOME/.clap/"
cp qimist-standalone run-qimist.sh MANUAL.html "$share/"
chmod +x "$share/qimist-standalone" "$share/run-qimist.sh"
ln -sf "$share/run-qimist.sh" "$bin/qimist"
cat > "$apps/qimist.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Qimist
Comment=Sing or beatbox into your mic and turn it into music
Exec=$share/run-qimist.sh
Terminal=false
Categories=AudioVideo;Audio;Music;
DESKTOP

echo "Qimist installed:"
echo "  VST3  ~/.vst3/Qimist.vst3"
echo "  CLAP  ~/.clap/Qimist.clap"
echo "  App   'Qimist' in your app menu, or run: qimist"
command -v pw-jack >/dev/null || echo "Note: the app needs PipeWire's JACK support (package pipewire-jack-audio-connection-kit or pipewire-jack)."
