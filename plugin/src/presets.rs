//! Presets: the sound + note-output settings, without the mic/detection setup
//! (that belongs to your room and microphone, not to a sound).
//!
//! User presets are JSON files in the config folder; factory presets are
//! defined here in plain (human) values and converted when listed.

use crate::params::VoxParams;
use nih_plug::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Parameters a preset never touches.
const EXCLUDED: &[&str] = &["gate", "sens", "denoise", "hum", "echocancel", "focus", "range", "latency", "monitor", "channel", "kicknote", "snarenote", "hatnote", "drumch", "exprcc", "loopbars", "loopmetro", "loopcount", "loopquant", "tempo"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub category: String,
    /// Param id → normalized value. Missing params load their default.
    pub params: BTreeMap<String, f32>,
    #[serde(skip)]
    pub factory: bool,
}

pub fn user_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("Qimist").join("presets"))
}

fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{}.json", clean.trim())
}

/// Factory presets first, then the user's (sorted by name).
pub fn list(params: &VoxParams) -> Vec<Preset> {
    let mut out = factory(params);
    let mut user: Vec<Preset> = user_dir()
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str::<Preset>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    user.sort_by_key(|p| p.name.to_lowercase());
    out.extend(user);
    out
}

pub fn save(p: &Preset) -> std::io::Result<PathBuf> {
    let dir = user_dir().ok_or_else(|| std::io::Error::other("no config folder"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(&p.name));
    std::fs::write(&path, serde_json::to_string_pretty(p).map_err(std::io::Error::other)?)?;
    Ok(path)
}

pub fn delete(name: &str) -> std::io::Result<()> {
    let dir = user_dir().ok_or_else(|| std::io::Error::other("no config folder"))?;
    std::fs::remove_file(dir.join(file_name(name)))
}

/// Snapshot of the current settings.
pub fn capture(params: &VoxParams, name: &str) -> Preset {
    let values = params
        .param_map()
        .into_iter()
        .filter(|(id, _, _)| !EXCLUDED.contains(&id.as_str()))
        .map(|(id, ptr, _)| (id, unsafe { ptr.unmodulated_normalized_value() }))
        .collect();
    Preset { name: name.to_string(), author: String::new(), category: "User".into(), params: values, factory: false }
}

/// Apply through the host so automation/undo see the change.
pub fn apply(params: &VoxParams, ctx: &dyn GuiContext, p: &Preset) {
    for (id, ptr, _) in params.param_map() {
        if EXCLUDED.contains(&id.as_str()) {
            continue;
        }
        unsafe {
            let v = p.params.get(&id).copied().unwrap_or_else(|| ptr.default_normalized_value()).clamp(0.0, 1.0);
            if (ptr.unmodulated_normalized_value() - v).abs() > 1e-6 {
                ctx.raw_begin_set_parameter(ptr);
                ctx.raw_set_parameter_normalized(ptr, v);
                ctx.raw_end_set_parameter(ptr);
            }
        }
    }
}

/// Names of the parameters that differ from the preset.
pub fn modified_params(params: &VoxParams, p: &Preset) -> Vec<String> {
    params
        .param_map()
        .into_iter()
        .filter(|(id, _, _)| !EXCLUDED.contains(&id.as_str()))
        .filter(|(id, ptr, _)| unsafe {
            let want = p.params.get(id).copied().unwrap_or_else(|| ptr.default_normalized_value());
            (ptr.unmodulated_normalized_value() - want).abs() > 1e-4
        })
        .map(|(_, ptr, _)| unsafe { ptr.name().to_string() })
        .collect()
}

/// True when the current settings differ from the preset.
pub fn is_modified(params: &VoxParams, p: &Preset) -> bool {
    params.param_map().into_iter().filter(|(id, _, _)| !EXCLUDED.contains(&id.as_str())).any(|(id, ptr, _)| unsafe {
        let want = p.params.get(&id).copied().unwrap_or_else(|| ptr.default_normalized_value());
        (ptr.unmodulated_normalized_value() - want).abs() > 1e-4
    })
}

// ─────────────────────────────── factory sounds ──────────────────────────────
//
// Plain values: enums are variant indices, bools 0/1.
// Wave: 0 Sine 1 Triangle 2 Saw 3 Square 4 Noise · FX target: 0 Synth 1 Voice 2 Both
// Mode: 0 Melody 1 Drums 2 Chords · Kit: 0 808 1 Tight 2 Lo-Fi · Range: 0 Low 1 Mid 2 High
// Chord: 0 Triad 1 7th 2 9th 3 Sus2 4 Sus4 5 Power 6 Oct · Arp rate: 0 1/4 1 1/8 2 1/8T 3 1/16 4 1/16T 5 1/32
// Arp pattern: 0 Up 1 Down 2 Up-Down 3 Random
// Filter: 0 LP12 1 LP24 2 BP 3 HP · LFO shape: 0 Sine 1 Tri 2 Square 3 S&H · LFO target: 0 Pitch 1 Filter 2 Amp
// Delay: 0 1/4 1 1/8. 2 1/8 3 1/4T 4 1/8T 5 1/16 6 1/2

type Plain = &'static [(&'static str, f32)];

const FACTORY: &[(&str, &str, Plain)] = &[
    ("Init", "Basic", &[]),
    (
        "Neon Hum Lead",
        "Lead",
        &[
            ("wave_1", 2.0), ("level_1", 0.8), ("oct_1", 0.0), ("detune_1", 0.0),
            ("wave_2", 2.0), ("level_2", 0.7), ("oct_2", 0.0), ("detune_2", 12.0),
            ("wave_3", 3.0), ("level_3", 0.3), ("oct_3", -1.0), ("detune_3", 0.0),
            ("cutoff", 3200.0), ("reso", 0.45), ("envamt", 0.55), ("glide", 60.0),
            ("attack", 5.0), ("decay", 400.0), ("sustain", 0.7), ("release", 300.0),
            ("chrate", 0.6), ("chdepth", 0.6), ("chmix", 0.45),
            ("eqlow", 1.0), ("eq250", 0.0), ("eq1k", -1.0), ("eq4k", 3.0), ("eqhigh", 3.0),
            ("rvsize", 0.65), ("rvdecay", 2.4), ("rvmix", 0.28), ("snap", 1.0),
            ("unison", 3.0), ("unidetune", 14.0), ("uniwidth", 0.8), ("dlytime", 1.0), ("dlymix", 0.18), ("dlyfb", 0.3),
        ],
    ),
    (
        "Auto-Tune Snap Lead",
        "Lead",
        &[
            ("snap", 1.0), ("hyst", 50.0), ("minnote", 60.0), ("bend", 0.0),
            ("wave_1", 2.0), ("level_1", 0.8),
            ("wave_2", 3.0), ("level_2", 0.5), ("detune_2", 5.0), ("oct_2", 0.0),
            ("wave_3", 0.0), ("level_3", 0.4), ("oct_3", -1.0),
            ("cutoff", 4500.0), ("reso", 0.3), ("envamt", 0.4), ("glide", 15.0),
            ("attack", 3.0), ("decay", 300.0), ("sustain", 0.8), ("release", 200.0),
            ("chmix", 0.3), ("rvmix", 0.22), ("eq4k", 3.0), ("eqhigh", 2.0),
        ],
    ),
    (
        "Whistle Flute",
        "Lead",
        &[
            ("wave_1", 0.0), ("level_1", 0.9), ("oct_1", 0.0),
            ("wave_2", 1.0), ("level_2", 0.3), ("oct_2", 1.0), ("detune_2", 0.0),
            ("wave_3", 4.0), ("level_3", 0.05), ("oct_3", 0.0),
            ("cutoff", 5000.0), ("reso", 0.1), ("envamt", 0.2), ("glide", 45.0),
            ("attack", 40.0), ("decay", 300.0), ("sustain", 0.85), ("release", 250.0),
            ("chrate", 0.4), ("chdepth", 0.3), ("chmix", 0.25),
            ("rvsize", 0.6), ("rvdecay", 2.2), ("rvmix", 0.3), ("eq1k", 2.0), ("eqlow", -2.0),
        ],
    ),
    (
        "Warm Bass Hum",
        "Bass",
        &[
            ("octave", -1.0),
            ("wave_1", 2.0), ("level_1", 0.9), ("oct_1", 0.0),
            ("wave_2", 3.0), ("level_2", 0.7), ("oct_2", -1.0), ("detune_2", 0.0),
            ("wave_3", 0.0), ("level_3", 0.6), ("oct_3", -1.0), ("detune_3", 0.0),
            ("cutoff", 600.0), ("reso", 0.3), ("envamt", 0.45), ("glide", 25.0),
            ("attack", 3.0), ("decay", 250.0), ("sustain", 0.8), ("release", 120.0),
            ("chmix", 0.1), ("eqlow", 4.0), ("eq250", 2.0), ("eq1k", -2.0), ("eq4k", -3.0), ("eqhigh", -4.0),
            ("rvsize", 0.3), ("rvmix", 0.08), ("fxtarget", 0.0),
        ],
    ),
    (
        "Analog Brass",
        "Keys & Brass",
        &[
            ("wave_1", 2.0), ("level_1", 0.9), ("oct_1", 0.0), ("detune_1", 0.0),
            ("wave_2", 2.0), ("level_2", 0.8), ("oct_2", 0.0), ("detune_2", -9.0),
            ("wave_3", 3.0), ("level_3", 0.3), ("oct_3", -1.0),
            ("cutoff", 900.0), ("reso", 0.25), ("envamt", 0.8), ("glide", 20.0),
            ("attack", 45.0), ("decay", 500.0), ("sustain", 0.65), ("release", 250.0),
            ("chmix", 0.2), ("eq250", 2.0), ("eq1k", 2.0), ("eqhigh", -1.0),
            ("rvsize", 0.5), ("rvdecay", 1.8), ("rvmix", 0.22),
        ],
    ),
    (
        "Glass Bells",
        "Keys & Brass",
        &[
            ("octave", 1.0),
            ("wave_1", 0.0), ("level_1", 0.8), ("oct_1", 0.0),
            ("wave_2", 1.0), ("level_2", 0.5), ("oct_2", 1.0), ("detune_2", 3.0),
            ("wave_3", 0.0), ("level_3", 0.35), ("oct_3", 2.0), ("detune_3", -4.0),
            ("cutoff", 9000.0), ("reso", 0.2), ("envamt", 0.2), ("glide", 0.0),
            ("attack", 2.0), ("decay", 1200.0), ("sustain", 0.15), ("release", 1800.0),
            ("chrate", 0.3), ("chdepth", 0.5), ("chmix", 0.35),
            ("eqlow", -2.0), ("eq4k", 2.0), ("eqhigh", 4.0),
            ("rvsize", 0.85), ("rvdecay", 5.0), ("rvpre", 35.0), ("rvmix", 0.42),
        ],
    ),
    (
        "Dreamy Pad",
        "Pad",
        &[
            ("snap", 1.0),
            ("wave_1", 2.0), ("level_1", 0.6), ("oct_1", 0.0), ("detune_1", -12.0),
            ("wave_2", 2.0), ("level_2", 0.6), ("oct_2", 0.0), ("detune_2", 12.0),
            ("wave_3", 1.0), ("level_3", 0.5), ("oct_3", 1.0), ("detune_3", 0.0),
            ("cutoff", 1800.0), ("reso", 0.2), ("envamt", 0.25), ("glide", 150.0),
            ("attack", 900.0), ("decay", 2000.0), ("sustain", 0.85), ("release", 3500.0),
            ("chrate", 0.25), ("chdepth", 0.85), ("chmix", 0.6),
            ("eqlow", -1.0), ("eqhigh", 2.0),
            ("rvsize", 0.95), ("rvdecay", 8.0), ("rvdamp", 0.5), ("rvpre", 40.0), ("rvwidth", 1.0), ("rvmix", 0.55),
            ("unison", 5.0), ("unidetune", 18.0), ("uniwidth", 1.0), ("filtmode", 1.0), ("lfotarget", 1.0), ("lforate", 0.15), ("lfodepth", 0.25),
        ],
    ),
    (
        "Chiptune",
        "Retro",
        &[
            ("snap", 1.0), ("hyst", 45.0), ("velcurve", 0.0),
            ("wave_1", 3.0), ("level_1", 0.85), ("oct_1", 0.0), ("detune_1", 0.0),
            ("wave_2", 3.0), ("level_2", 0.4), ("oct_2", 1.0), ("detune_2", 0.0),
            ("wave_3", 4.0), ("level_3", 0.0),
            ("cutoff", 16000.0), ("reso", 0.0), ("envamt", 0.0), ("glide", 0.0),
            ("attack", 1.0), ("decay", 120.0), ("sustain", 0.7), ("release", 60.0),
            ("chmix", 0.0), ("eq4k", 2.0), ("rvsize", 0.3), ("rvmix", 0.1),
        ],
    ),
    (
        "Supersaw Anthem",
        "Lead",
        &[
            ("snap", 1.0), ("unison", 7.0), ("unidetune", 22.0), ("uniwidth", 1.0), ("filtmode", 1.0),
            ("wave_1", 2.0), ("level_1", 0.85), ("oct_1", 0.0), ("detune_1", 0.0),
            ("wave_2", 2.0), ("level_2", 0.5), ("oct_2", 1.0), ("detune_2", 0.0),
            ("wave_3", 0.0), ("level_3", 0.4), ("oct_3", -1.0),
            ("cutoff", 5000.0), ("reso", 0.2), ("envamt", 0.35), ("glide", 40.0),
            ("attack", 10.0), ("decay", 600.0), ("sustain", 0.8), ("release", 400.0),
            ("chmix", 0.0), ("dlytime", 1.0), ("dlyfb", 0.4), ("dlymix", 0.22), ("dlytone", 4000.0),
            ("rvsize", 0.8), ("rvdecay", 3.0), ("rvmix", 0.3), ("eq4k", 2.0), ("synvol", 0.55),
        ],
    ),
    (
        "Echo Whistle",
        "Lead",
        &[
            ("wave_1", 0.0), ("level_1", 0.9), ("wave_2", 1.0), ("level_2", 0.25), ("oct_2", 1.0), ("level_3", 0.0),
            ("cutoff", 6000.0), ("envamt", 0.1), ("glide", 50.0), ("attack", 30.0), ("sustain", 0.9), ("release", 300.0),
            ("lfotarget", 0.0), ("lforate", 5.5), ("lfodepth", 0.15),
            ("dlytime", 1.0), ("dlyfb", 0.5), ("dlymix", 0.35), ("dlypp", 1.0), ("dlytone", 3500.0),
            ("rvsize", 0.7), ("rvmix", 0.25),
        ],
    ),
    (
        "Wobble Bass",
        "Bass",
        &[
            ("octave", -1.0), ("filtmode", 1.0), ("drive", 0.5), ("unison", 2.0), ("unidetune", 8.0), ("uniwidth", 0.3),
            ("wave_1", 2.0), ("level_1", 0.9), ("wave_2", 3.0), ("level_2", 0.7), ("oct_2", -1.0), ("level_3", 0.0),
            ("cutoff", 300.0), ("reso", 0.6), ("envamt", 0.2), ("glide", 30.0),
            ("attack", 3.0), ("decay", 300.0), ("sustain", 0.9), ("release", 120.0),
            ("lfoshape", 0.0), ("lfotarget", 1.0), ("lforate", 3.0), ("lfodepth", 0.8),
            ("chmix", 0.0), ("eqlow", 3.0), ("rvmix", 0.05), ("fxtarget", 0.0),
        ],
    ),
    (
        "Sing-a-Chord Pad",
        "Chords",
        &[
            ("mode", 2.0), ("chordtype", 1.0), ("chordspread", 1.0), ("chordbass", 1.0), ("hold", 120.0), ("minnote", 90.0),
            ("wave_1", 2.0), ("level_1", 0.55), ("oct_1", 0.0), ("detune_1", -10.0),
            ("wave_2", 2.0), ("level_2", 0.55), ("oct_2", 0.0), ("detune_2", 10.0),
            ("wave_3", 1.0), ("level_3", 0.4), ("oct_3", 1.0),
            ("cutoff", 2000.0), ("reso", 0.15), ("envamt", 0.2), ("glide", 80.0),
            ("attack", 400.0), ("decay", 1500.0), ("sustain", 0.8), ("release", 1800.0),
            ("chrate", 0.3), ("chdepth", 0.7), ("chmix", 0.5), ("synvol", 0.55),
            ("rvsize", 0.9), ("rvdecay", 6.0), ("rvmix", 0.45), ("rvpre", 30.0),
        ],
    ),
    (
        "Power Chord Riff",
        "Chords",
        &[
            ("mode", 2.0), ("chordtype", 5.0), ("chordbass", 1.0), ("chordfollow", 1.0), ("chordhold", 0.06), ("chordring", 0.0), ("chordsmooth", 0.0), ("octave", -1.0),
            ("wave_1", 2.0), ("level_1", 0.9), ("wave_2", 3.0), ("level_2", 0.6), ("oct_2", 0.0), ("detune_2", 8.0),
            ("wave_3", 2.0), ("level_3", 0.5), ("oct_3", -1.0),
            ("cutoff", 1400.0), ("reso", 0.35), ("envamt", 0.6), ("glide", 10.0),
            ("attack", 4.0), ("decay", 350.0), ("sustain", 0.75), ("release", 200.0),
            ("chmix", 0.15), ("eq250", 2.0), ("eq4k", 3.0), ("rvsize", 0.4), ("rvmix", 0.15),
        ],
    ),
    (
        "Arp Dreams",
        "Arp",
        &[
            ("mode", 2.0), ("chordtype", 1.0), ("arp", 1.0), ("arprate", 3.0), ("arppat", 2.0), ("arpoct", 2.0), ("arpgate", 0.5),
            ("wave_1", 2.0), ("level_1", 0.7), ("wave_2", 3.0), ("level_2", 0.4), ("oct_2", 1.0), ("detune_2", 6.0),
            ("wave_3", 0.0), ("level_3", 0.3), ("oct_3", -1.0),
            ("cutoff", 2600.0), ("reso", 0.4), ("envamt", 0.5), ("glide", 0.0),
            ("attack", 2.0), ("decay", 220.0), ("sustain", 0.3), ("release", 300.0),
            ("chrate", 0.5), ("chdepth", 0.5), ("chmix", 0.35),
            ("rvsize", 0.8), ("rvdecay", 3.5), ("rvmix", 0.35), ("rvpre", 25.0),
        ],
    ),
    (
        "Retro Arp Bass",
        "Arp",
        &[
            ("arp", 1.0), ("arprate", 3.0), ("arppat", 0.0), ("arpoct", 2.0), ("arpgate", 0.35), ("octave", -1.0),
            ("wave_1", 3.0), ("level_1", 0.85), ("wave_2", 2.0), ("level_2", 0.5), ("oct_2", -1.0), ("level_3", 0.0),
            ("cutoff", 700.0), ("reso", 0.55), ("envamt", 0.7), ("glide", 0.0),
            ("attack", 1.0), ("decay", 150.0), ("sustain", 0.2), ("release", 80.0),
            ("chmix", 0.0), ("eqlow", 3.0), ("rvmix", 0.08), ("fxtarget", 0.0),
        ],
    ),
    (
        "Robot Voice",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 0.0), ("atamount", 1.0), ("snap", 1.0), ("synth", 0.0), ("fxtarget", 1.0),
            ("chrate", 0.4), ("chdepth", 0.3), ("chmix", 0.2), ("eq250", -2.0), ("eq4k", 3.0), ("eqhigh", 2.0),
            ("rvsize", 0.5), ("rvdecay", 1.6), ("rvmix", 0.18),
        ],
    ),
    (
        "Natural Pitch Fix",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 60.0), ("atamount", 0.8), ("synth", 0.0), ("fxtarget", 1.0),
            ("chmix", 0.0), ("eq4k", 1.0), ("rvsize", 0.55), ("rvdecay", 1.8), ("rvmix", 0.15),
        ],
    ),
    (
        "Choir Harmony",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 30.0), ("atamount", 0.9), ("harmony", 1.0), ("harmlevel", 0.7), ("synth", 0.0), ("fxtarget", 1.0),
            ("chrate", 0.25), ("chdepth", 0.6), ("chmix", 0.35), ("eqlow", -2.0), ("eqhigh", 2.0),
            ("rvsize", 0.9), ("rvdecay", 4.5), ("rvpre", 30.0), ("rvmix", 0.4),
        ],
    ),
    (
        "Deep Voice",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 40.0), ("atamount", 0.0), ("formant", -4.0), ("synth", 0.0), ("fxtarget", 1.0),
            ("chmix", 0.0), ("eqlow", 2.0), ("eq4k", -1.0), ("rvsize", 0.4), ("rvmix", 0.1),
        ],
    ),
    (
        "Tiny Voice",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 25.0), ("atamount", 0.6), ("formant", 5.0), ("synth", 0.0), ("fxtarget", 1.0),
            ("chmix", 0.0), ("eqlow", -3.0), ("eqhigh", 2.0), ("rvsize", 0.3), ("rvmix", 0.12),
        ],
    ),
    (
        "Monster Tune",
        "Voice FX",
        &[
            ("autotune", 1.0), ("atspeed", 0.0), ("atamount", 1.0), ("snap", 1.0), ("formant", -6.5), ("harmony", 1.0), ("harmlevel", 0.4),
            ("synth", 0.0), ("fxtarget", 1.0), ("eqlow", 3.0), ("rvsize", 0.8), ("rvdecay", 3.0), ("rvmix", 0.25),
        ],
    ),
    (
        "Expressive Lead",
        "Lead",
        &[
            ("voicedyn", 0.85), ("wave_1", 2.0), ("level_1", 0.85), ("wave_2", 2.0), ("level_2", 0.5), ("detune_2", 9.0), ("level_3", 0.0),
            ("cutoff", 900.0), ("reso", 0.4), ("envamt", 0.15), ("glide", 0.05), ("attack", 8.0), ("sustain", 0.9), ("release", 220.0),
            ("unison", 3.0), ("unidetune", 10.0), ("chmix", 0.2), ("dlymix", 0.18), ("rvmix", 0.15), ("fxtarget", 0.0),
        ],
    ),
    (
        "Beatbox 808",
        "Drums",
        &[
            ("mode", 1.0), ("drumkit", 0.0), ("drumvol", 0.9), ("drumsens", 0.6), ("drumtune", 0.0), ("drumdecay", 1.2),
            ("eqlow", 3.0), ("rvsize", 0.4), ("rvmix", 0.12), ("chmix", 0.0), ("fxtarget", 0.0),
        ],
    ),
    (
        "Tight Funk Kit",
        "Drums",
        &[
            ("mode", 1.0), ("drumkit", 1.0), ("drumvol", 0.85), ("drumsens", 0.7), ("drumtune", 2.0), ("drumdecay", 0.8),
            ("eq4k", 2.0), ("rvsize", 0.3), ("rvmix", 0.08), ("chmix", 0.0), ("fxtarget", 0.0),
        ],
    ),
    (
        "Trap Beatbox",
        "Drums",
        &[
            ("mode", 1.0), ("drumkit", 4.0), ("drumvol", 0.9), ("drumsens", 0.65), ("drumdecay", 1.3), ("kicklvl", 1.0), ("hatlvl", 0.75),
            ("eqlow", 2.0), ("eqhigh", 2.0), ("rvsize", 0.35), ("rvmix", 0.1), ("chmix", 0.0), ("fxtarget", 0.0), ("outgain", 1.5),
        ],
    ),
    (
        "Acoustic Room Kit",
        "Drums",
        &[
            ("mode", 1.0), ("drumkit", 3.0), ("drumvol", 0.85), ("drumdecay", 1.0),
            ("rvsize", 0.5), ("rvdecay", 1.2), ("rvdamp", 0.5), ("rvmix", 0.2), ("chmix", 0.0), ("fxtarget", 0.0),
        ],
    ),
    (
        "Lo-Fi Beats",
        "Drums",
        &[
            ("mode", 1.0), ("drumkit", 2.0), ("drumvol", 0.9), ("drumtune", -2.0), ("drumdecay", 0.9), ("hatlvl", 0.7),
            ("chrate", 0.3), ("chmix", 0.25), ("eqlow", 2.0), ("eq4k", -2.0), ("eqhigh", -6.0),
            ("rvsize", 0.6), ("rvdamp", 0.7), ("rvmix", 0.25), ("fxtarget", 0.0),
        ],
    ),
];

fn factory(params: &VoxParams) -> Vec<Preset> {
    let map = params.param_map();
    FACTORY
        .iter()
        .map(|(name, category, plain)| {
            let values = plain
                .iter()
                .filter_map(|(id, v)| {
                    let (_, ptr, _) = map.iter().find(|(pid, _, _)| pid == id)?;
                    Some((id.to_string(), unsafe { ptr.preview_normalized(*v) }))
                })
                .collect();
            Preset { name: name.to_string(), author: "Qimist".into(), category: category.to_string(), params: values, factory: true }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_ids_exist() {
        let params = VoxParams::default();
        let map = params.param_map();
        for (name, _, plain) in FACTORY {
            for (id, _) in plain.iter() {
                assert!(map.iter().any(|(pid, _, _)| pid == id), "{name}: unknown param id {id}");
            }
        }
    }

    #[test]
    fn roundtrip_json() {
        let params = VoxParams::default();
        let p = capture(&params, "Test");
        let back: Preset = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.params, p.params);
        assert!(!is_modified(&params, &back));
    }
}

#[cfg(test)]
mod init_tests {
    use super::*;

    #[test]
    fn fresh_params_match_init() {
        let params = VoxParams::default();
        let init = factory(&params).into_iter().find(|p| p.name == "Init").unwrap();
        let diff: Vec<String> = params
            .param_map()
            .into_iter()
            .filter(|(id, _, _)| !EXCLUDED.contains(&id.as_str()))
            .filter(|(id, ptr, _)| unsafe {
                let want = init.params.get(id).copied().unwrap_or_else(|| ptr.default_normalized_value());
                (ptr.unmodulated_normalized_value() - want).abs() > 1e-4
            })
            .map(|(id, _, _)| id)
            .collect();
        assert!(diff.is_empty(), "differs: {diff:?}");
    }
}
