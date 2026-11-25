# Qimist

Sing, hum or whistle into your mic. Qimist detects the pitch, the notes and
the key, and sends them out as MIDI. It can also record what you sang and
export it as a `.mid` file. Formats: **VST3**, **CLAP** and a **standalone app**.

Design reference: `design/qimist-ui.html` (open it in a browser).

## Layout

```
crates/qimist-dsp/   pure-Rust DSP (no plugin deps): pitch, notes, key, MIDI file
plugin/                nih-plug plugin + egui editor + standalone binary
xtask/                 bundler (cargo xtask bundle …)
design/                interactive HTML mockup
```

## Detection pipeline

Detection is built to stay accurate on a cheap laptop mic:

1. **Front end:** 65 Hz high-pass, 50/60 Hz hum notches, 2.8 kHz low-pass, decimated to about 16 kHz.
2. **Denoiser:** STFT spectral subtraction. It learns the fan or room noise automatically.
3. **pYIN:** 100-threshold YIN that gives several pitch candidates, each with a probability.
4. **HMM + fixed-lag Viterbi:** picks the most likely pitch path, which removes octave jumps and glitches.
5. **Adaptive gate:** follows the room's noise floor, so breathing doesn't trigger notes.
6. **Note tracker:**
   - median filtering
   - hysteresis, so vibrato doesn't flutter between notes
   - minimum note length
   - re-articulation (onset) detection
   - consonant bridging
   - scale snap
7. **Key detector:** Krumhansl-Schmuckler correlation over sung pitch classes. Older material fades out, so the key can follow a song that changes key.

## Setup (Fedora)

```sh
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# System libraries for audio (ALSA/JACK) and the GUI (X11/OpenGL)
sudo dnf install -y gcc alsa-lib-devel pipewire-jack-audio-connection-kit-devel \
    libX11-devel libXcursor-devel libxcb-devel xcb-util-wm-devel \
    mesa-libGL-devel libxkbcommon-x11-devel
```

## Build

```sh
cargo test -p qimist-dsp --release -- --nocapture   # accuracy table
cargo xtask bundle qimist --release                  # → target/bundled/Qimist.vst3 / .clap
./run-qimist.sh                                      # standalone, auto-connects your mic (PipeWire)
cargo run -p qimist --release --bin qimist-standalone            # standalone with JACK/PipeWire
cargo run -p qimist --release --bin qimist-standalone -- --backend alsa
```

To install, copy `target/bundled/Qimist.vst3` into `~/.vst3/` and
`Qimist.clap` into `~/.clap/`.

## Test with your own voice

```sh
arecord -f S16_LE -r 48000 -c 1 -d 10 me.wav     # record 10 s with the laptop mic
cargo run -p qimist-dsp --release --example analyze -- me.wav --snap
```

This prints every detected note and the key, and writes `me.mid`.

## In a DAW

1. Put Qimist on an **audio track** whose input is your mic.
2. Turn the track's input monitoring on.
3. Route the plugin's **MIDI output** to an instrument track. Reaper, Bitwig and Cubase support this directly.
4. If your DAW can't route MIDI out of an effect (for example Ableton), use **RECORD**, then **EXPORT .MID**. The file goes to `~/Music/Qimist/`; drag it into your DAW.

## Roadmap

- [x] Phase 1–2: detection engine, MIDI out, tuner, key & scale, scale snap, recorder, `.mid` export
- [x] Phase 3: synth (3 oscillators, filter, ADSR, glide), chorus, 5-band EQ, reverb, FX target
- [x] Phase 4: beatbox → drum kit (onset detection, kick/snare/hat classifier, per-user TRAIN, built-in 808/Tight/Lo-Fi kits, MIDI ch 10)
- [x] Presets: 12 factory sounds + save/load/delete your own (`~/.config/Qimist/presets/*.json`; mic setup is not stored)
- [x] Phase 5: poly synth, diatonic chords (7 types, inversions, open voicing, bass), tempo-synced arpeggiator, voice auto-tune + 3rd/5th harmony
- [x] Phase 6: unison (up to 7) + drive, LP12/LP24/BP/HP filter, LFO, tempo-synced stereo/ping-pong delay, master limiter + meters, 5 drum kits, per-pad level/MIDI note/channel, custom WAV samples (`~/Music/Qimist/Samples`), host-grid arp sync, voice CALIBRATE wizard, UI scale 75–150%, knob tooltips + right-click type-in
- [x] Phase 7: formant-preserving voice auto-tune & harmony (TD-PSOLA) + FORMANT shift, VOICE DYNAMICS (loudness → synth filter/level), loudness → MIDI expression CC
- [x] Phase 8: low-latency voice monitoring — standalone asks PipeWire for 128-sample buffers (`QIMIST_BUFFER=256 ./run-qimist.sh` if it crackles), LOW LAT auto-tune (fast YIN tracker + 10 ms PSOLA look-ahead, ~10 ms instead of ~52 ms)
- [x] Chords v2: CHORD PADS screen (7 chords of the key, click & hold to play), smart chord follower (`follow.rs`: MELODY harmonising or ROOT, STEADY hold time so passing notes don't change chords, change NOW/BEAT/BAR, LET RING, smooth voice leading), friendly styles (POP/SOUL/JAZZ/DREAMY/FOLK/ROCK/OCTAVE)
- [x] Phase 9: LOOP STATION (`looper.rs`, LOOPER tab on the centre screen): 4 layers over 1/2/4/8 bars, REC/overdub/mute/clear/undo, count-in + metronome click, snap 1/8 · 1/16, host-tempo sync (or TEMPO in the standalone), loops saved with the project, export loop .mid
- [x] Loop fix for laptop speakers: echo canceller on the analysis path (`aec.rs`, two-path NLMS at 16 kHz, ECHO toggle in INPUT) so the loop playing through the speakers doesn't confuse detection; notes already ringing when a take starts are recorded too
- [x] Phase 10: piano-roll editing (`editor/noteedit.rs`): the finished take (NOTES tab) and each loop layer (click a lane in LOOPER) are editable: click / Shift-click / box select, drag to move (snaps to the grid, up/down = pitch), drag the right edge to resize, double-click to add, Delete, ↑/↓ transpose (Shift = octave), ←/→ nudge, Q / SNAP quantize, Ctrl+Z / Ctrl+Y undo/redo
- [x] First-run guide (`editor/guide.rs`): a 10-step spotlight tour, opens once automatically (marker file `guide-seen` in the config folder) and from the **?** button in the header
- [x] Releases: `scripts/package.sh` builds Linux (`.tar.gz` + `install.sh`) and Windows (cross-compiled `.zip`, NSIS installer when `makensis` is installed) into `dist/`; `packaging/macos/build-pkg.sh` builds the universal macOS `.pkg`/`.zip` on a Mac; `.github/workflows/release.yml` builds all three on GitHub. User manual: `docs/MANUAL.html`. The Windows/macOS standalone opens the default mic (patched in vendor/nih-plug cpal backend)
- [x] Song mode: four scenes A–D per project (`SongData`, persisted as `song`), scene switches wait for the end of the pass, COPY to the next empty scene, SONG arrangement (steps × repeats, REPEAT), PLAY SONG, EXPORT SONG
- [x] Drums v2: attack/body analysis + voicing feature (kick vs snare on laptop mics), hit map
