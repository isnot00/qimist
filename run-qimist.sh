#!/usr/bin/env bash
# Start the Qimist standalone app and connect the default microphone to it.
set -euo pipefail
cd "$(dirname "$0")"
# Installed copies keep the binary next to this script; the source tree has it in target/release.
bin=./qimist-standalone
[[ -x $bin ]] || bin=target/release/qimist-standalone

# Small audio buffers = low monitoring latency (PipeWire defaults to 1024 samples ≈ 21 ms).
# Override with e.g. QIMIST_BUFFER=256 if you hear crackles.
PIPEWIRE_LATENCY="${QIMIST_BUFFER:-128}/48000" pw-jack "$bin" "$@" &
app=$!

# Wait for the app's JACK ports, then wire the first capture device to them.
for _ in $(seq 50); do
    pw-link -i | grep -q '^qimist:input_1$' && break
    sleep 0.1
done
mic=$(pw-link -o | grep -m1 -E '^alsa_input.*:capture_(FL|MONO|1)$' | sed 's/:capture_.*//' || true)
if [[ -n "$mic" ]]; then
    if pw-link -o | grep -q "^$mic:capture_FL$"; then
        pw-link "$mic:capture_FL" qimist:input_1 || true
        pw-link "$mic:capture_FR" qimist:input_2 || true
    else
        port=$(pw-link -o | grep -m1 "^$mic:capture_")
        pw-link "$port" qimist:input_1 || true
        pw-link "$port" qimist:input_2 || true
    fi
    echo "Qimist: microphone $mic connected"
else
    echo "Qimist: no microphone found — connect one with qpwgraph or pw-link" >&2
fi
wait "$app"
