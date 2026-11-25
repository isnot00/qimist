//! Standalone app: opens the microphone directly, no DAW needed.
//!
//!   qimist-standalone                       # JACK / PipeWire-JACK
//!   qimist-standalone --backend alsa --input-device default
//!
//! On Windows and macOS it uses the default mic and speakers.

// No console window behind the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use nih_plug::prelude::*;

fn main() {
    nih_export_standalone::<qimist::Qimist>();
}
