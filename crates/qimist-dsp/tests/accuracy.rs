//! End-to-end accuracy on synthetic singing, clean and through a simulated
//! laptop mic with fan noise. Run with `cargo test -p qimist-dsp --release -- --nocapture`
//! to see the metrics table.

use qimist_dsp::testsig::{render, Segment, SynthOpts};
use qimist_dsp::{EngineConfig, EngineEvent, PitchConfig, PitchTracker, VoiceEngine};

const SR: f32 = 48_000.0;

fn melody() -> Vec<Segment> {
    use Segment as S;
    vec![
        S::rest(0.4),
        S::note(57.0, 0.45), // A3
        S::rest(0.12),
        S::note(60.0, 0.4), // C4
        S::note(64.0, 0.35), // E4 (legato)
        S::note(67.0, 0.35), // G4
        S::rest(0.15),
        S::note(69.0, 0.6), // A4
        S::rest(0.2),
        S::note(45.0, 0.5), // A2, low male voice (110 Hz)
        S::rest(0.15),
        S::note(81.0, 0.4), // A5 (880 Hz)
        S::rest(0.15),
        S::note(52.0, 0.4), // E3
        S::note(55.0, 0.4), // G3
        S::rest(0.4),
    ]
}

struct Metrics {
    recall: f32,
    false_alarm: f32,
    gpe: f32,
    mean_abs_cents: f32,
}

/// Background chatter: `talkers` voices with speech-like short, gliding
/// syllables, each `level_db` relative to the singer, through the same mic.
fn crowd(len: usize, talkers: u32, level_db: f32) -> Vec<f32> {
    let mut mix = vec![0.0f32; len];
    for k in 0..talkers {
        let mut rng = 0x9E37_79B9u32.wrapping_mul(k + 11);
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as f32 / u32::MAX as f32
        };
        let mut segs = Vec::new();
        let mut t = 0.0;
        while t < len as f32 / SR {
            let d = 0.08 + 0.25 * rnd();
            segs.push(Segment::note(45.0 + 20.0 * rnd(), d));
            if rnd() < 0.35 {
                segs.push(Segment::rest(0.05 + 0.2 * rnd()));
            }
            t += d;
        }
        let opts = SynthOpts { vibrato_cents: 80.0, vibrato_hz: 3.0, jitter_cents: 30.0, laptop_mic: true, level: 0.3 * 10f32.powf(level_db / 20.0), seed: 100 + k, snr_db: None };
        let (v, _) = render(SR, &segs, opts);
        for (m, x) in mix.iter_mut().zip(v) {
            *m += x;
        }
    }
    mix
}

fn pitch_metrics(opts: SynthOpts, cfg: PitchConfig) -> Metrics {
    pitch_metrics_with(opts, cfg, None)
}

fn pitch_metrics_with(opts: SynthOpts, cfg: PitchConfig, background: Option<(u32, f32)>) -> Metrics {
    pitch_metrics_from(opts, cfg, background, 0.0)
}

/// `score_from_s`: ignore frames before this time (e.g. before the singer's first
/// note, when focus mode hasn't heard your voice yet).
fn pitch_metrics_from(opts: SynthOpts, cfg: PitchConfig, background: Option<(u32, f32)>, score_from_s: f32) -> Metrics {
    let (mut sig, truth) = render(SR, &melody(), opts);
    if let Some((n, db)) = background {
        for (s, b) in sig.iter_mut().zip(crowd(truth.len(), n, db)) {
            *s += b;
        }
    }
    let mut tr = PitchTracker::new(SR as f64, cfg);
    let lat = tr.latency_samples();
    let (mut voiced_truth, mut hit, mut silent_truth, mut fa, mut gross, mut both, mut cents_sum) = (0, 0, 0, 0, 0, 0, 0.0f32);
    for (i, &x) in sig.iter().enumerate() {
        let Some(f) = tr.push(x) else { continue };
        let Some(t) = i.checked_sub(lat) else { continue };
        if (t as f32) < score_from_s * SR {
            continue;
        }
        // Skip frames within 40 ms of a voicing boundary in the ground truth.
        // "Silent" must also be past the simulated room echo (up to 41 ms), which
        // a real mic genuinely hears.
        let guard = (0.04 * SR) as usize;
        let lo = t.saturating_sub(guard);
        let hi = (t + guard).min(truth.len() - 1);
        let all_voiced = truth[lo..=hi].iter().all(Option::is_some);
        let echo = if opts.laptop_mic { (0.05 * SR) as usize } else { 0 };
        let all_silent = truth[t.saturating_sub(guard + echo)..=hi].iter().all(Option::is_none);
        if all_voiced {
            voiced_truth += 1;
            if f.voiced {
                hit += 1;
                both += 1;
                let c = 100.0 * (f.midi - truth[t].unwrap());
                if c.abs() > 50.0 {
                    gross += 1;
                } else {
                    cents_sum += c.abs();
                }
            }
        } else if all_silent {
            silent_truth += 1;
            if f.voiced {
                fa += 1;
            }
        }
    }
    Metrics {
        recall: hit as f32 / voiced_truth.max(1) as f32,
        false_alarm: fa as f32 / silent_truth.max(1) as f32,
        gpe: gross as f32 / both.max(1) as f32,
        mean_abs_cents: cents_sum / (both - gross).max(1) as f32,
    }
}

fn conditions() -> Vec<(&'static str, SynthOpts, PitchConfig)> {
    let base = SynthOpts::default();
    vec![
        ("clean", base, PitchConfig::default()),
        ("laptop, SNR 25 dB", SynthOpts { laptop_mic: true, snr_db: Some(25.0), ..base }, PitchConfig::laptop_mic()),
        ("laptop, SNR 15 dB", SynthOpts { laptop_mic: true, snr_db: Some(15.0), seed: 7, ..base }, PitchConfig::laptop_mic()),
        ("laptop, SNR 10 dB", SynthOpts { laptop_mic: true, snr_db: Some(10.0), seed: 3, ..base }, PitchConfig::laptop_mic()),
    ]
}

#[test]
fn pitch_accuracy_table() {
    println!("\n{:<20} {:>8} {:>8} {:>8} {:>8}", "condition", "recall", "falseAl", "GPE", "|cents|");
    for (name, opts, cfg) in conditions() {
        let m = pitch_metrics(opts, cfg);
        println!("{:<20} {:>7.1}% {:>7.1}% {:>7.2}% {:>8.1}", name, m.recall * 100.0, m.false_alarm * 100.0, m.gpe * 100.0, m.mean_abs_cents);
        // Gross pitch errors (> 50 cents) must stay rare even in noise.
        assert!(m.gpe < 0.03, "{name}: GPE {:.2}%", m.gpe * 100.0);
        assert!(m.recall > 0.85, "{name}: recall {:.1}%", m.recall * 100.0);
        assert!(m.false_alarm < 0.05, "{name}: false alarms {:.1}%", m.false_alarm * 100.0);
    }
}

#[test]
fn crowd_rejection() {
    let opts = SynthOpts { laptop_mic: true, snr_db: Some(25.0), seed: 5, ..Default::default() };
    println!("\n{:<26} {:>8} {:>8} {:>8}", "crowd (3 talkers, -18 dB)", "recall", "falseAl", "GPE");
    let results: Vec<(f32, Metrics)> = [0.0f32, 0.3, 0.5, 0.8, 1.0]
        .into_iter()
        // Scored from the singer's first note (0.4 s): before that, focus can't know your level.
        .map(|focus| (focus, pitch_metrics_from(opts, PitchConfig { focus, ..PitchConfig::laptop_mic() }, Some((3, -18.0)), 0.4)))
        .collect();
    for (focus, m) in &results {
        println!("{:<26} {:>7.1}% {:>7.1}% {:>7.2}%", format!("focus {:.0}%", focus * 100.0), m.recall * 100.0, m.false_alarm * 100.0, m.gpe * 100.0);
    }
    for (focus, m) in results {
        if focus >= 0.5 {
            assert!(m.false_alarm < 0.05, "focus {focus}: false alarms {:.1}%", m.false_alarm * 100.0);
            assert!(m.recall > 0.85, "focus {focus}: recall {:.1}%", m.recall * 100.0);
            assert!(m.gpe < 0.03, "focus {focus}: GPE {:.2}%", m.gpe * 100.0);
        }
    }
}

/// Informational: how far crowd rejection goes (no assertions).
#[test]
fn crowd_limits_report() {
    let opts = SynthOpts { laptop_mic: true, snr_db: Some(25.0), seed: 5, ..Default::default() };
    println!("\n{:<34} {:>8} {:>8} {:>8}", "harder crowds (focus 50%)", "recall", "falseAl", "GPE");
    for (talkers, db) in [(3u32, -18.0f32), (6, -18.0), (3, -12.0), (6, -12.0), (3, -6.0)] {
        let m = pitch_metrics_from(opts, PitchConfig { focus: 0.5, ..PitchConfig::laptop_mic() }, Some((talkers, db)), 0.4);
        println!("{:<34} {:>7.1}% {:>7.1}% {:>7.2}%", format!("{talkers} talkers each {db} dB"), m.recall * 100.0, m.false_alarm * 100.0, m.gpe * 100.0);
    }
}

#[test]
fn notes_match_melody_on_laptop_mic() {
    let opts = SynthOpts { laptop_mic: true, snr_db: Some(15.0), ..Default::default() };
    let (sig, _) = render(SR, &melody(), opts);
    let cfg = EngineConfig { pitch: PitchConfig::laptop_mic(), ..Default::default() };
    let mut eng = VoiceEngine::new(SR as f64, cfg);
    let mut ons = Vec::new();
    for &x in &sig {
        eng.push(x, &mut |e| {
            if let EngineEvent::NoteOn { note, .. } = e {
                ons.push(note);
            }
        });
    }
    let expected: Vec<u8> = melody().iter().filter_map(|s| s.midi.map(|m| m as u8)).collect();
    assert_eq!(ons, expected);
}
