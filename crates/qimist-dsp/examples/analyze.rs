//! Analyse a WAV file with the Qimist engine: prints detected notes and key,
//! and writes a .mid next to the input.
//!
//! Record yourself with the laptop mic and test:
//!   arecord -f S16_LE -r 48000 -c 1 -d 10 me.wav
//!   cargo run -p qimist-dsp --release --example analyze -- me.wav [--snap] [--fast|--accurate]

use qimist_dsp::{note_name, EngineConfig, EngineEvent, LatencyMode, PitchConfig, Recorder, VoiceEngine};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: analyze <file.wav> [--snap] [--fast|--accurate] [--clean-mic]");
        std::process::exit(2);
    };
    let flag = |f: &str| args.iter().any(|a| a == f);

    let mut reader = hound::WavReader::open(path).expect("cannot open wav");
    let spec = reader.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 * scale).collect()
        }
    };
    let mono: Vec<f32> = samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    let sr = spec.sample_rate as f64;

    let mut pitch = if flag("--clean-mic") { PitchConfig::default() } else { PitchConfig::laptop_mic() };
    if flag("--fast") {
        pitch.latency = LatencyMode::Fast;
    }
    if flag("--accurate") {
        pitch.latency = LatencyMode::Accurate;
    }
    let cfg = EngineConfig { pitch, snap_enabled: flag("--snap"), ..Default::default() };
    let mut eng = VoiceEngine::new(sr, cfg);
    let mut rec = Recorder::default();

    println!("{} — {:.1} s @ {} Hz, latency {:.1} ms", path, mono.len() as f64 / sr, spec.sample_rate, eng.latency_samples() as f64 / sr * 1000.0);
    for (i, &x) in mono.iter().enumerate() {
        let now = i as f64 / sr;
        eng.push(x, &mut |e| match e {
            EngineEvent::NoteOn { note, velocity, delay_samples } => {
                let t = now - delay_samples as f64 / sr;
                println!("{t:7.3}s  ON  {:<4} vel {velocity}", note_name(note));
                rec.note_on(t, note, velocity, 0);
            }
            EngineEvent::NoteOff { note, delay_samples } => rec.note_off(now - delay_samples as f64 / sr, note),
            EngineEvent::PitchBend { .. } => {}
        });
    }
    rec.close_all(mono.len() as f64 / sr);

    if let Some(k) = eng.key_estimate() {
        println!("\nKey: {} ({:.0}%)  alt: {} {:.0}%, {} {:.0}%", k.key.name(), k.confidence * 100.0,
            k.alternatives[0].0.name(), k.alternatives[0].1 * 100.0, k.alternatives[1].0.name(), k.alternatives[1].1 * 100.0);
    }
    let out = std::path::Path::new(path).with_extension("mid");
    std::fs::write(&out, rec.to_smf(120.0, true, 0)).expect("write .mid");
    println!("{} notes → {}", rec.notes().len(), out.display());
}
