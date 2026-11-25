use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;
use qimist_dsp::notes::VelocityCurve;
use qimist_dsp::fx::{ChorusSettings, DelaySettings, ReverbSettings};
use qimist_dsp::synth::{FilterMode, LfoSettings, LfoShape, LfoTarget, OscSettings, SynthSettings, Wave};
use parking_lot::RwLock;
use qimist_dsp::arp::{ArpPattern, ArpSettings};
use qimist_dsp::autotune::AutoTuneSettings;
use qimist_dsp::chords::ChordType;
use qimist_dsp::drums::{DrumModel, Kit, KitSettings};
use qimist_dsp::looper::{LoopData, SongData};
use qimist_dsp::{EngineConfig, LatencyMode, NoteConfig, PitchConfig, Scale, ScaleKind};

pub const EDITOR_W: u32 = 1200;
pub const EDITOR_H: u32 = 900;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[name = "Melody"]
    Melody,
    #[name = "Drums"]
    Drums,
    #[name = "Chords"]
    Chords,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceRange {
    #[name = "Low (50-700 Hz)"]
    Low,
    #[name = "Mid (65-1100 Hz)"]
    Mid,
    #[name = "High (110-1500 Hz)"]
    High,
}

impl VoiceRange {
    pub fn hz(self) -> (f32, f32) {
        match self {
            VoiceRange::Low => (50.0, 700.0),
            VoiceRange::Mid => (65.0, 1100.0),
            VoiceRange::High => (110.0, 1500.0),
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Latency {
    #[name = "Fast"]
    Fast,
    #[name = "Balanced"]
    Balanced,
    #[name = "Accurate"]
    Accurate,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VelCurve {
    #[name = "Fixed"]
    Fixed,
    #[name = "Linear"]
    Linear,
    #[name = "Soft"]
    Soft,
    #[name = "Hard"]
    Hard,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    #[name = "Auto"]
    Auto,
    #[name = "Lock"]
    Lock,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantize {
    #[name = "Off"]
    Off,
    #[name = "1/4"]
    Q4,
    #[name = "1/8"]
    Q8,
    #[name = "1/16"]
    Q16,
    #[name = "1/32"]
    Q32,
}

impl Quantize {
    pub fn division(self) -> u32 {
        match self {
            Quantize::Off => 0,
            Quantize::Q4 => 4,
            Quantize::Q8 => 8,
            Quantize::Q16 => 16,
            Quantize::Q32 => 32,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleChoice {
    #[name = "Major"]
    Major,
    #[name = "Minor"]
    NaturalMinor,
    #[name = "Harmonic Minor"]
    HarmonicMinor,
    #[name = "Melodic Minor"]
    MelodicMinor,
    #[name = "Dorian"]
    Dorian,
    #[name = "Phrygian"]
    Phrygian,
    #[name = "Lydian"]
    Lydian,
    #[name = "Mixolydian"]
    Mixolydian,
    #[name = "Locrian"]
    Locrian,
    #[name = "Major Pentatonic"]
    MajorPentatonic,
    #[name = "Minor Pentatonic"]
    MinorPentatonic,
    #[name = "Blues"]
    Blues,
    #[name = "Chromatic"]
    Chromatic,
}

impl ScaleChoice {
    pub fn kind(self) -> ScaleKind {
        ScaleKind::ALL[self as usize]
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum KitChoice {
    #[name = "808"]
    Eight08,
    #[name = "Tight"]
    Tight,
    #[name = "Lo-Fi"]
    LoFi,
    #[name = "Acoustic"]
    Acoustic,
    #[name = "Trap"]
    Trap,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterChoice {
    #[name = "LP 12"]
    Lp12,
    #[name = "LP 24"]
    Lp24,
    #[name = "Band"]
    Bp,
    #[name = "High"]
    Hp,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoShapeChoice {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Square"]
    Square,
    #[name = "Sample & Hold"]
    SampleHold,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoTargetChoice {
    #[name = "Pitch"]
    Pitch,
    #[name = "Filter"]
    Filter,
    #[name = "Amp"]
    Amp,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayDiv {
    #[name = "1/4"]
    Q4,
    #[name = "1/8 dotted"]
    Q8D,
    #[name = "1/8"]
    Q8,
    #[name = "1/4 triplet"]
    Q4T,
    #[name = "1/8 triplet"]
    Q8T,
    #[name = "1/16"]
    Q16,
    #[name = "1/2"]
    Q2,
}

impl DelayDiv {
    /// Length in quarter notes.
    pub fn beats(self) -> f32 {
        match self {
            DelayDiv::Q4 => 1.0,
            DelayDiv::Q8D => 0.75,
            DelayDiv::Q8 => 0.5,
            DelayDiv::Q4T => 2.0 / 3.0,
            DelayDiv::Q8T => 1.0 / 3.0,
            DelayDiv::Q16 => 0.25,
            DelayDiv::Q2 => 2.0,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordChoice {
    #[name = "Triad"]
    Triad,
    #[name = "7th"]
    Seventh,
    #[name = "9th"]
    Ninth,
    #[name = "Sus2"]
    Sus2,
    #[name = "Sus4"]
    Sus4,
    #[name = "Power"]
    Power,
    #[name = "Octaves"]
    Octaves,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordFollowChoice {
    #[name = "Melody"]
    Melody,
    #[name = "Root"]
    Root,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopBars {
    #[name = "1 Bar"]
    B1,
    #[name = "2 Bars"]
    B2,
    #[name = "4 Bars"]
    B4,
    #[name = "8 Bars"]
    B8,
}

impl LoopBars {
    pub fn beats(self) -> f64 {
        match self {
            LoopBars::B1 => 4.0,
            LoopBars::B2 => 8.0,
            LoopBars::B4 => 16.0,
            LoopBars::B8 => 32.0,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopQuant {
    #[name = "Off"]
    Off,
    #[name = "1/8"]
    Q8,
    #[name = "1/16"]
    Q16,
}

impl LoopQuant {
    pub fn beats(self) -> f64 {
        match self {
            LoopQuant::Off => 0.0,
            LoopQuant::Q8 => 0.5,
            LoopQuant::Q16 => 0.25,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordSyncChoice {
    #[name = "Free"]
    Free,
    #[name = "Beat"]
    Beat,
    #[name = "Bar"]
    Bar,
}

impl ChordChoice {
    pub fn kind(self) -> ChordType {
        ChordType::ALL[self as usize]
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpRate {
    #[name = "1/4"]
    Q4,
    #[name = "1/8"]
    Q8,
    #[name = "1/8T"]
    Q8T,
    #[name = "1/16"]
    Q16,
    #[name = "1/16T"]
    Q16T,
    #[name = "1/32"]
    Q32,
}

impl ArpRate {
    fn steps_per_whole(self) -> f32 {
        match self {
            ArpRate::Q4 => 4.0,
            ArpRate::Q8 => 8.0,
            ArpRate::Q8T => 12.0,
            ArpRate::Q16 => 16.0,
            ArpRate::Q16T => 24.0,
            ArpRate::Q32 => 32.0,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpPatternChoice {
    #[name = "Up"]
    Up,
    #[name = "Down"]
    Down,
    #[name = "Up-Down"]
    UpDown,
    #[name = "Random"]
    Random,
}

#[derive(Params)]
pub struct VoxParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Beatbox training examples, saved with the project.
    #[persist = "drum-model"]
    pub drum_model: Arc<RwLock<DrumModel>>,
    /// Name of the last loaded/saved preset.
    #[persist = "preset-name"]
    pub preset_name: Arc<RwLock<String>>,
    /// Custom drum samples (file paths; empty = built-in sound), saved with the project.
    #[persist = "drum-samples"]
    pub drum_samples: Arc<RwLock<[String; 3]>>,
    /// Loop station from before scenes (read once into scene A).
    #[persist = "loops"]
    pub loops: Arc<RwLock<LoopData>>,
    /// Loop station: the four scenes and the song arrangement.
    #[persist = "song"]
    pub song: Arc<RwLock<SongData>>,

    #[id = "mode"]
    pub mode: EnumParam<Mode>,

    // ── Input / detection ────────────────────────────────────────────
    #[id = "gate"]
    pub gate: FloatParam,
    #[id = "sens"]
    pub sensitivity: FloatParam,
    #[id = "denoise"]
    pub denoise: FloatParam,
    #[id = "hum"]
    pub hum_notch: BoolParam,
    /// Remove Qimist's own sound (from laptop speakers) before detection.
    #[id = "echocancel"]
    pub echo_cancel: BoolParam,
    #[id = "focus"]
    pub focus: FloatParam,
    #[id = "range"]
    pub range: EnumParam<VoiceRange>,
    #[id = "latency"]
    pub latency: EnumParam<Latency>,

    // ── Note output ──────────────────────────────────────────────────
    #[id = "minnote"]
    pub min_note: FloatParam,
    #[id = "hold"]
    pub hold: FloatParam,
    #[id = "hyst"]
    pub hysteresis: FloatParam,
    #[id = "octave"]
    pub octave: IntParam,
    #[id = "transp"]
    pub transpose: IntParam,
    #[id = "bend"]
    pub pitch_bend: BoolParam,
    #[id = "bendrng"]
    pub bend_range: IntParam,
    #[id = "velcurve"]
    pub vel_curve: EnumParam<VelCurve>,
    #[id = "channel"]
    pub midi_channel: IntParam,
    #[id = "quant"]
    pub quantize: EnumParam<Quantize>,

    // ── Key & scale ──────────────────────────────────────────────────
    #[id = "snap"]
    pub snap: BoolParam,
    #[id = "keymode"]
    pub key_mode: EnumParam<KeyMode>,
    #[id = "keyroot"]
    pub key_root: IntParam,
    #[id = "keyscale"]
    pub key_scale: EnumParam<ScaleChoice>,

    // ── Output ───────────────────────────────────────────────────────
    #[id = "monitor"]
    pub monitor: FloatParam,

    // ── Sound engine ─────────────────────────────────────────────────
    #[id = "synth"]
    pub synth_on: BoolParam,
    #[id = "synvol"]
    pub synth_volume: FloatParam,
    #[nested(array, group = "Oscillator")]
    pub oscs: [OscParams; 3],
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub resonance: FloatParam,
    #[id = "envamt"]
    pub env_amount: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,

    #[id = "chrate"]
    pub chorus_rate: FloatParam,
    #[id = "chdepth"]
    pub chorus_depth: FloatParam,
    #[id = "chmix"]
    pub chorus_mix: FloatParam,

    #[id = "eqlow"]
    pub eq_low: FloatParam,
    #[id = "eq250"]
    pub eq_250: FloatParam,
    #[id = "eq1k"]
    pub eq_1k: FloatParam,
    #[id = "eq4k"]
    pub eq_4k: FloatParam,
    #[id = "eqhigh"]
    pub eq_high: FloatParam,

    #[id = "rvsize"]
    pub rv_size: FloatParam,
    #[id = "rvdecay"]
    pub rv_decay: FloatParam,
    #[id = "rvdamp"]
    pub rv_damp: FloatParam,
    #[id = "rvpre"]
    pub rv_predelay: FloatParam,
    #[id = "rvwidth"]
    pub rv_width: FloatParam,
    #[id = "rvmix"]
    pub rv_mix: FloatParam,

    #[id = "fxtarget"]
    pub fx_target: EnumParam<FxTarget>,

    // ── Drums ────────────────────────────────────────────────────────
    #[id = "drumkit"]
    pub drum_kit: EnumParam<KitChoice>,
    #[id = "drumvol"]
    pub drum_volume: FloatParam,
    #[id = "drumsens"]
    pub drum_sens: FloatParam,
    #[id = "drumtune"]
    pub drum_tune: IntParam,
    #[id = "drumdecay"]
    pub drum_decay: FloatParam,
    #[id = "kicklvl"]
    pub kick_level: FloatParam,
    #[id = "snarelvl"]
    pub snare_level: FloatParam,
    #[id = "hatlvl"]
    pub hat_level: FloatParam,
    #[id = "kicknote"]
    pub kick_note: IntParam,
    #[id = "snarenote"]
    pub snare_note: IntParam,
    #[id = "hatnote"]
    pub hat_note: IntParam,
    #[id = "drumch"]
    pub drum_channel: IntParam,

    // ── Chords ───────────────────────────────────────────────────────
    #[id = "chordtype"]
    pub chord_type: EnumParam<ChordChoice>,
    #[id = "chordinv"]
    pub chord_inversion: IntParam,
    #[id = "chordspread"]
    pub chord_spread: BoolParam,
    #[id = "chordbass"]
    pub chord_bass: BoolParam,
    #[id = "chordfollow"]
    pub chord_follow: EnumParam<ChordFollowChoice>,
    /// How long a new note must be held before the chord changes.
    #[id = "chordhold"]
    pub chord_hold: FloatParam,
    #[id = "chordsync"]
    pub chord_sync: EnumParam<ChordSyncChoice>,
    #[id = "chordring"]
    pub chord_ring: BoolParam,
    #[id = "chordsmooth"]
    pub chord_smooth: BoolParam,

    // ── Loop station ─────────────────────────────────────────────────
    #[id = "loopbars"]
    pub loop_bars: EnumParam<LoopBars>,
    #[id = "loopmetro"]
    pub loop_metronome: BoolParam,
    #[id = "loopcount"]
    pub loop_count_in: BoolParam,
    #[id = "loopquant"]
    pub loop_quantize: EnumParam<LoopQuant>,
    /// Tempo of the standalone app (in a DAW the host's tempo is used).
    #[id = "tempo"]
    pub tempo: IntParam,

    // ── Arpeggiator ──────────────────────────────────────────────────
    #[id = "arp"]
    pub arp_on: BoolParam,
    #[id = "arprate"]
    pub arp_rate: EnumParam<ArpRate>,
    #[id = "arppat"]
    pub arp_pattern: EnumParam<ArpPatternChoice>,
    #[id = "arpoct"]
    pub arp_octaves: IntParam,
    #[id = "arpgate"]
    pub arp_gate: FloatParam,

    // ── Voice auto-tune / harmony ────────────────────────────────────
    #[id = "autotune"]
    pub autotune_on: BoolParam,
    #[id = "atspeed"]
    pub autotune_speed: FloatParam,
    #[id = "atamount"]
    pub autotune_amount: FloatParam,
    #[id = "harmony"]
    pub harmony_on: BoolParam,
    #[id = "harmlevel"]
    pub harmony_level: FloatParam,
    #[id = "formant"]
    pub formant: FloatParam,
    /// Track the voice directly (~10 ms) instead of waiting for the smoothed detector.
    #[id = "atlowlat"]
    pub autotune_low_latency: BoolParam,

    // ── Voice expression ─────────────────────────────────────────────
    /// Synth filter/level follow the singer's loudness.
    #[id = "voicedyn"]
    pub voice_dynamics: FloatParam,
    /// MIDI CC that carries the singer's loudness (0 = off).
    #[id = "exprcc"]
    pub expr_cc: IntParam,

    // ── Synth: filter mode, drive, unison, LFO ───────────────────────
    #[id = "filtmode"]
    pub filter_mode: EnumParam<FilterChoice>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "unison"]
    pub unison: IntParam,
    #[id = "unidetune"]
    pub unison_detune: FloatParam,
    #[id = "uniwidth"]
    pub unison_width: FloatParam,
    #[id = "lfoshape"]
    pub lfo_shape: EnumParam<LfoShapeChoice>,
    #[id = "lforate"]
    pub lfo_rate: FloatParam,
    #[id = "lfodepth"]
    pub lfo_depth: FloatParam,
    #[id = "lfotarget"]
    pub lfo_target: EnumParam<LfoTargetChoice>,

    // ── Delay & master ───────────────────────────────────────────────
    #[id = "dlytime"]
    pub delay_div: EnumParam<DelayDiv>,
    #[id = "dlyfb"]
    pub delay_feedback: FloatParam,
    #[id = "dlytone"]
    pub delay_tone: FloatParam,
    #[id = "dlymix"]
    pub delay_mix: FloatParam,
    #[id = "dlypp"]
    pub delay_ping_pong: BoolParam,
    #[id = "outgain"]
    pub output_gain: FloatParam,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveChoice {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Saw"]
    Saw,
    #[name = "Square"]
    Square,
    #[name = "Noise"]
    Noise,
}

impl WaveChoice {
    pub fn wave(self) -> Wave {
        match self {
            WaveChoice::Sine => Wave::Sine,
            WaveChoice::Triangle => Wave::Triangle,
            WaveChoice::Saw => Wave::Saw,
            WaveChoice::Square => Wave::Square,
            WaveChoice::Noise => Wave::Noise,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTarget {
    #[name = "Synth"]
    Synth,
    #[name = "Voice"]
    Voice,
    #[name = "Both"]
    Both,
}

#[derive(Params)]
pub struct OscParams {
    #[id = "wave"]
    pub wave: EnumParam<WaveChoice>,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "oct"]
    pub octave: IntParam,
    #[id = "detune"]
    pub detune: FloatParam,
}

impl OscParams {
    fn new(wave: WaveChoice, level: f32, octave: i32, detune: f32) -> Self {
        OscParams {
            wave: EnumParam::new("Wave", wave),
            level: FloatParam::new("Level", level, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            octave: IntParam::new("Octave", octave, IntRange::Linear { min: -2, max: 2 })
                .with_value_to_string(Arc::new(|v| if v > 0 { format!("+{v} oct") } else { format!("{v} oct") })),
            detune: FloatParam::new("Detune", detune, FloatRange::Linear { min: -50.0, max: 50.0 })
                .with_value_to_string(Arc::new(|v| format!("{v:+.0}ct"))),
        }
    }
}

fn pct(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn time_ms(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) }).with_value_to_string(Arc::new(|v| {
        if v >= 1000.0 {
            format!("{:.1}s", v / 1000.0)
        } else {
            format!("{v:.0}ms")
        }
    }))
}

fn eq_gain(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: -12.0, max: 12.0 }).with_value_to_string(Arc::new(|v| format!("{v:+.0}dB")))
}

fn note_param(name: &str, default: i32) -> IntParam {
    IntParam::new(name, default, IntRange::Linear { min: 0, max: 127 })
        .with_value_to_string(Arc::new(|v| format!("{v} {}", qimist_dsp::note_name(v.clamp(0, 127) as u8))))
}

fn hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else {
        format!("{v:.0}")
    }
}

fn ms(v: f32) -> String {
    format!("{v:.0}ms")
}

impl Default for VoxParams {
    fn default() -> Self {
        VoxParams {
            editor_state: EguiState::from_size(EDITOR_W, EDITOR_H),
            drum_model: Arc::new(RwLock::new(DrumModel::default())),
            preset_name: Arc::new(RwLock::new("Init".to_string())),
            loops: Arc::new(RwLock::new(LoopData::default())),
            song: Arc::new(RwLock::new(SongData::default())),
            drum_samples: Arc::new(RwLock::new(Default::default())),
            mode: EnumParam::new("Mode", Mode::Melody),

            gate: FloatParam::new("Gate", -55.0, FloatRange::Linear { min: -80.0, max: -20.0 })
                .with_value_to_string(Arc::new(|v| format!("{v:.0}dB"))),
            sensitivity: FloatParam::new("Sensitivity", 0.65, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            denoise: FloatParam::new("Denoise", 0.7, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            hum_notch: BoolParam::new("Hum Notch", true),
            echo_cancel: BoolParam::new("Speaker Echo Cancel", true),
            focus: pct("Focus", 0.3),
            range: EnumParam::new("Voice Range", VoiceRange::Mid),
            latency: EnumParam::new("Latency", Latency::Balanced),

            min_note: FloatParam::new("Min Note", 45.0, FloatRange::Skewed { min: 15.0, max: 200.0, factor: FloatRange::skew_factor(-1.0) })
                .with_value_to_string(Arc::new(ms)),
            hold: FloatParam::new("Hold", 50.0, FloatRange::Skewed { min: 20.0, max: 300.0, factor: FloatRange::skew_factor(-1.0) })
                .with_value_to_string(Arc::new(ms)),
            hysteresis: FloatParam::new("Hysteresis", 30.0, FloatRange::Linear { min: 0.0, max: 60.0 })
                .with_value_to_string(Arc::new(|v| format!("{v:.0}¢"))),
            octave: IntParam::new("Octave", 0, IntRange::Linear { min: -3, max: 3 })
                .with_value_to_string(Arc::new(|v| if v > 0 { format!("+{v}") } else { v.to_string() })),
            transpose: IntParam::new("Transpose", 0, IntRange::Linear { min: -12, max: 12 })
                .with_value_to_string(Arc::new(|v| if v > 0 { format!("+{v}st") } else { format!("{v}st") })),
            pitch_bend: BoolParam::new("Pitch Bend Out", false),
            bend_range: IntParam::new("Bend Range", 2, IntRange::Linear { min: 1, max: 12 }).with_unit(" st"),
            vel_curve: EnumParam::new("Velocity Curve", VelCurve::Soft),
            midi_channel: IntParam::new("MIDI Channel", 1, IntRange::Linear { min: 1, max: 16 }),
            quantize: EnumParam::new("Export Quantize", Quantize::Off),

            snap: BoolParam::new("Scale Snap", false),
            key_mode: EnumParam::new("Key Mode", KeyMode::Auto),
            key_root: IntParam::new("Key Root", 9, IntRange::Linear { min: 0, max: 11 })
                .with_value_to_string(Arc::new(|v| qimist_dsp::NOTE_NAMES[v.clamp(0, 11) as usize].to_string())),
            key_scale: EnumParam::new("Key Scale", ScaleChoice::NaturalMinor),

            monitor: FloatParam::new("Voice Monitor", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            synth_on: BoolParam::new("Synth On", true),
            synth_volume: pct("Synth Volume", 0.7).with_smoother(SmoothingStyle::Linear(20.0)),
            oscs: [
                OscParams::new(WaveChoice::Saw, 0.85, 0, 0.0),
                OscParams::new(WaveChoice::Square, 0.6, -1, 7.0),
                OscParams::new(WaveChoice::Sine, 0.3, 1, -5.0),
            ],
            cutoff: FloatParam::new("Cutoff", 2400.0, FloatRange::Skewed { min: 40.0, max: 18_000.0, factor: FloatRange::skew_factor(-2.0) })
                .with_value_to_string(Arc::new(hz)),
            resonance: pct("Resonance", 0.35),
            env_amount: pct("Env Amount", 0.5),
            glide: time_ms("Glide", 30.0, 0.0, 1000.0),
            attack: time_ms("Attack", 8.0, 1.0, 3000.0),
            decay: time_ms("Decay", 320.0, 5.0, 5000.0),
            sustain: pct("Sustain", 0.6),
            release: time_ms("Release", 450.0, 5.0, 5000.0),

            chorus_rate: FloatParam::new("Chorus Rate", 0.8, FloatRange::Skewed { min: 0.05, max: 5.0, factor: FloatRange::skew_factor(-1.0) })
                .with_value_to_string(Arc::new(|v| format!("{v:.2}Hz"))),
            chorus_depth: pct("Chorus Depth", 0.55),
            chorus_mix: pct("Chorus Mix", 0.4),

            eq_low: eq_gain("EQ Low", 2.0),
            eq_250: eq_gain("EQ 250 Hz", 0.0),
            eq_1k: eq_gain("EQ 1 kHz", -2.0),
            eq_4k: eq_gain("EQ 4 kHz", 1.0),
            eq_high: eq_gain("EQ High", 2.0),

            rv_size: pct("Reverb Size", 0.7),
            rv_decay: FloatParam::new("Reverb Decay", 2.8, FloatRange::Skewed { min: 0.2, max: 12.0, factor: FloatRange::skew_factor(-1.5) })
                .with_value_to_string(Arc::new(|v| format!("{v:.1}s"))),
            rv_damp: pct("Reverb Damp", 0.4),
            rv_predelay: FloatParam::new("Reverb Pre-delay", 20.0, FloatRange::Linear { min: 0.0, max: 100.0 }).with_value_to_string(Arc::new(|v| format!("{v:.0}ms"))),
            rv_width: pct("Reverb Width", 0.9),
            rv_mix: pct("Reverb Mix", 0.3),

            fx_target: EnumParam::new("FX Target", FxTarget::Both),

            drum_kit: EnumParam::new("Drum Kit", KitChoice::Eight08),
            drum_volume: pct("Drum Volume", 0.8).with_smoother(SmoothingStyle::Linear(20.0)),
            drum_sens: pct("Hit Sensitivity", 0.6),
            drum_tune: IntParam::new("Drum Tune", 0, IntRange::Linear { min: -12, max: 12 })
                .with_value_to_string(Arc::new(|v| if v > 0 { format!("+{v}st") } else { format!("{v}st") })),
            drum_decay: FloatParam::new("Drum Decay", 1.0, FloatRange::Skewed { min: 0.4, max: 2.5, factor: FloatRange::skew_factor(-0.5) })
                .with_value_to_string(Arc::new(|v| format!("{v:.2}x"))),

            chord_type: EnumParam::new("Chord Type", ChordChoice::Triad),
            chord_inversion: IntParam::new("Chord Inversion", 0, IntRange::Linear { min: 0, max: 3 }),
            chord_spread: BoolParam::new("Open Voicing", false),
            chord_bass: BoolParam::new("Chord Bass Note", false),
            chord_follow: EnumParam::new("Chord Follow", ChordFollowChoice::Melody),
            chord_hold: FloatParam::new("Chord Change Hold", 0.2, FloatRange::Skewed { min: 0.0, max: 1.0, factor: FloatRange::skew_factor(-1.0) })
                .with_value_to_string(Arc::new(|v| format!("{:.0}ms", v * 1000.0)))
                .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches("ms").trim().parse::<f32>().ok().map(|v| v / 1000.0))),
            chord_sync: EnumParam::new("Chord Change Sync", ChordSyncChoice::Free),
            chord_ring: BoolParam::new("Let Chords Ring", true),
            chord_smooth: BoolParam::new("Smooth Voicing", true),
            loop_bars: EnumParam::new("Loop Length", LoopBars::B2),
            loop_metronome: BoolParam::new("Metronome", true),
            loop_count_in: BoolParam::new("Count-In", true),
            loop_quantize: EnumParam::new("Loop Quantize", LoopQuant::Q16),
            tempo: IntParam::new("Tempo", 120, IntRange::Linear { min: 40, max: 240 }).with_unit(" BPM"),

            arp_on: BoolParam::new("Arpeggiator", false),
            arp_rate: EnumParam::new("Arp Rate", ArpRate::Q16),
            arp_pattern: EnumParam::new("Arp Pattern", ArpPatternChoice::Up),
            arp_octaves: IntParam::new("Arp Octaves", 1, IntRange::Linear { min: 1, max: 4 }),
            arp_gate: FloatParam::new("Arp Gate", 0.6, FloatRange::Linear { min: 0.05, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            autotune_on: BoolParam::new("Auto-Tune", false),
            autotune_speed: FloatParam::new("Retune Speed", 15.0, FloatRange::Skewed { min: 0.0, max: 300.0, factor: FloatRange::skew_factor(-1.5) })
                .with_value_to_string(Arc::new(|v| if v < 0.5 { "ROBOT".to_string() } else { format!("{v:.0}ms") })),
            autotune_amount: pct("Auto-Tune Amount", 1.0),
            harmony_on: BoolParam::new("Voice Harmony", false),
            harmony_level: pct("Harmony Level", 0.6),
            formant: FloatParam::new("Formant", 0.0, FloatRange::Linear { min: -7.0, max: 7.0 })
                .with_step_size(0.1)
                .with_value_to_string(Arc::new(|v| if v.abs() < 0.05 { "NATURAL".to_string() } else { format!("{v:+.1}st") })),
            autotune_low_latency: BoolParam::new("Low-Latency Tune", true),
            voice_dynamics: pct("Voice Dynamics", 0.0),
            expr_cc: IntParam::new("Expression CC", 0, IntRange::Linear { min: 0, max: 119 })
                .with_value_to_string(Arc::new(|v| if v == 0 { "OFF".to_string() } else { format!("CC{v}") })),

            kick_level: pct("Kick Level", 1.0),
            snare_level: pct("Snare Level", 1.0),
            hat_level: pct("Hi-Hat Level", 1.0),
            kick_note: note_param("Kick Note", 36),
            snare_note: note_param("Snare Note", 38),
            hat_note: note_param("Hi-Hat Note", 42),
            drum_channel: IntParam::new("Drum MIDI Channel", 10, IntRange::Linear { min: 1, max: 16 }),

            filter_mode: EnumParam::new("Filter Type", FilterChoice::Lp12),
            drive: pct("Drive", 0.0),
            unison: IntParam::new("Unison Voices", 1, IntRange::Linear { min: 1, max: 7 }),
            unison_detune: FloatParam::new("Unison Detune", 12.0, FloatRange::Linear { min: 0.0, max: 50.0 })
                .with_value_to_string(Arc::new(|v| format!("{v:.0}ct"))),
            unison_width: pct("Unison Width", 0.7),
            lfo_shape: EnumParam::new("LFO Shape", LfoShapeChoice::Sine),
            lfo_rate: FloatParam::new("LFO Rate", 5.0, FloatRange::Skewed { min: 0.05, max: 20.0, factor: FloatRange::skew_factor(-1.5) })
                .with_value_to_string(Arc::new(|v| format!("{v:.2}Hz"))),
            lfo_depth: pct("LFO Depth", 0.0),
            lfo_target: EnumParam::new("LFO Target", LfoTargetChoice::Pitch),

            delay_div: EnumParam::new("Delay Time", DelayDiv::Q8D),
            delay_feedback: FloatParam::new("Delay Feedback", 0.35, FloatRange::Linear { min: 0.0, max: 0.95 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            delay_tone: FloatParam::new("Delay Tone", 5000.0, FloatRange::Skewed { min: 500.0, max: 16_000.0, factor: FloatRange::skew_factor(-1.5) })
                .with_value_to_string(Arc::new(hz)),
            delay_mix: pct("Delay Mix", 0.0),
            delay_ping_pong: BoolParam::new("Delay Ping-Pong", true),
            output_gain: FloatParam::new("Output Gain", 0.0, FloatRange::Linear { min: -24.0, max: 6.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}dB"))),
        }
    }
}

impl VoxParams {
    pub fn engine_config(&self) -> EngineConfig {
        let (fmin, fmax) = self.range.value().hz();
        let pitch = PitchConfig {
            fmin,
            fmax,
            latency: match self.latency.value() {
                Latency::Fast => LatencyMode::Fast,
                Latency::Balanced => LatencyMode::Balanced,
                Latency::Accurate => LatencyMode::Accurate,
            },
            denoise: self.denoise.value(),
            sensitivity: self.sensitivity.value(),
            gate_db: self.gate.value(),
            hum_notch: self.hum_notch.value(),
            focus: self.focus.value(),
        };
        let notes = NoteConfig {
            min_note_ms: self.min_note.value(),
            release_ms: self.hold.value(),
            hysteresis_cents: self.hysteresis.value(),
            transpose: self.octave.value() * 12 + self.transpose.value(),
            snap: None,
            pitch_bend: self.pitch_bend.value(),
            bend_range: self.bend_range.value() as f32,
            velocity_curve: match self.vel_curve.value() {
                VelCurve::Fixed => VelocityCurve::Fixed,
                VelCurve::Linear => VelocityCurve::Linear,
                VelCurve::Soft => VelocityCurve::Soft,
                VelCurve::Hard => VelocityCurve::Hard,
            },
            ..NoteConfig::default()
        };
        let locked_scale = (self.key_mode.value() == KeyMode::Lock)
            .then(|| Scale::new(self.key_root.value() as u8, self.key_scale.value().kind()));
        EngineConfig { pitch, notes, snap_enabled: self.snap.value(), locked_scale }
    }

    pub fn synth_settings(&self) -> SynthSettings {
        let osc = |o: &OscParams| OscSettings { wave: o.wave.value().wave(), level: o.level.value(), octave: o.octave.value(), detune_cents: o.detune.value() };
        SynthSettings {
            oscs: [osc(&self.oscs[0]), osc(&self.oscs[1]), osc(&self.oscs[2])],
            cutoff_hz: self.cutoff.value(),
            resonance: self.resonance.value(),
            env_amount: self.env_amount.value(),
            attack_s: self.attack.value() / 1000.0,
            decay_s: self.decay.value() / 1000.0,
            sustain: self.sustain.value(),
            release_s: self.release.value() / 1000.0,
            glide_s: self.glide.value() / 1000.0,
            volume: 1.0,
            filter_mode: match self.filter_mode.value() {
                FilterChoice::Lp12 => FilterMode::Lp12,
                FilterChoice::Lp24 => FilterMode::Lp24,
                FilterChoice::Bp => FilterMode::Bp,
                FilterChoice::Hp => FilterMode::Hp,
            },
            drive: self.drive.value(),
            unison: self.unison.value() as u8,
            unison_detune_cents: self.unison_detune.value(),
            unison_width: self.unison_width.value(),
            dynamics: self.voice_dynamics.value(),
            lfo: LfoSettings {
                shape: match self.lfo_shape.value() {
                    LfoShapeChoice::Sine => LfoShape::Sine,
                    LfoShapeChoice::Triangle => LfoShape::Triangle,
                    LfoShapeChoice::Square => LfoShape::Square,
                    LfoShapeChoice::SampleHold => LfoShape::SampleHold,
                },
                rate_hz: self.lfo_rate.value(),
                depth: self.lfo_depth.value(),
                target: match self.lfo_target.value() {
                    LfoTargetChoice::Pitch => LfoTarget::Pitch,
                    LfoTargetChoice::Filter => LfoTarget::Filter,
                    LfoTargetChoice::Amp => LfoTarget::Amp,
                },
            },
        }
    }

    pub fn eq_gains(&self) -> [f32; 5] {
        [self.eq_low.value(), self.eq_250.value(), self.eq_1k.value(), self.eq_4k.value(), self.eq_high.value()]
    }

    pub fn chorus_settings(&self) -> ChorusSettings {
        ChorusSettings { rate_hz: self.chorus_rate.value(), depth: self.chorus_depth.value(), mix: self.chorus_mix.value() }
    }

    pub fn reverb_settings(&self) -> ReverbSettings {
        ReverbSettings {
            size: self.rv_size.value(),
            decay_s: self.rv_decay.value(),
            damp: self.rv_damp.value(),
            predelay_s: self.rv_predelay.value() / 1000.0,
            width: self.rv_width.value(),
            mix: self.rv_mix.value(),
        }
    }

    pub fn kit_settings(&self) -> KitSettings {
        KitSettings {
            kit: match self.drum_kit.value() {
                KitChoice::Eight08 => Kit::Eight08,
                KitChoice::Tight => Kit::Tight,
                KitChoice::LoFi => Kit::LoFi,
                KitChoice::Acoustic => Kit::Acoustic,
                KitChoice::Trap => Kit::Trap,
            },
            tune: self.drum_tune.value() as f32,
            decay: self.drum_decay.value(),
            volume: 1.0,
            levels: [self.kick_level.value(), self.snare_level.value(), self.hat_level.value()],
        }
    }

    pub fn arp_settings(&self) -> ArpSettings {
        ArpSettings {
            pattern: match self.arp_pattern.value() {
                ArpPatternChoice::Up => ArpPattern::Up,
                ArpPatternChoice::Down => ArpPattern::Down,
                ArpPatternChoice::UpDown => ArpPattern::UpDown,
                ArpPatternChoice::Random => ArpPattern::Random,
            },
            steps_per_whole: self.arp_rate.value().steps_per_whole(),
            octaves: self.arp_octaves.value() as u8,
            gate: self.arp_gate.value(),
        }
    }

    pub fn autotune_settings(&self) -> AutoTuneSettings {
        AutoTuneSettings {
            enabled: self.autotune_on.value(),
            speed_ms: self.autotune_speed.value(),
            amount: self.autotune_amount.value(),
            harmony: self.harmony_on.value(),
            harmony_level: self.harmony_level.value(),
            formant_st: self.formant.value(),
            low_latency: self.autotune_low_latency.value(),
            gate_db: self.gate.value(),
        }
    }

    pub fn follow_settings(&self) -> qimist_dsp::follow::FollowSettings {
        use qimist_dsp::follow::{ChangeSync, FollowMode, FollowSettings};
        FollowSettings {
            mode: match self.chord_follow.value() {
                ChordFollowChoice::Melody => FollowMode::Melody,
                ChordFollowChoice::Root => FollowMode::Root,
            },
            hold_s: self.chord_hold.value(),
            sync: match self.chord_sync.value() {
                ChordSyncChoice::Free => ChangeSync::Free,
                ChordSyncChoice::Beat => ChangeSync::Beat,
                ChordSyncChoice::Bar => ChangeSync::Bar,
            },
            let_ring: self.chord_ring.value(),
            smooth: self.chord_smooth.value(),
            kind: self.chord_type.value().kind(),
            inversion: self.chord_inversion.value() as u8,
            spread: self.chord_spread.value(),
            bass: self.chord_bass.value(),
        }
    }

    /// The chord played for a sung `note` in chords mode.
    pub fn chord_for(&self, note: u8, scale: Option<Scale>) -> qimist_dsp::chords::Chord {
        qimist_dsp::chords::build(
            note,
            self.chord_type.value().kind(),
            scale,
            self.chord_inversion.value() as u8,
            self.chord_spread.value(),
            self.chord_bass.value(),
        )
    }

    pub fn delay_settings(&self, bpm: f32) -> DelaySettings {
        DelaySettings {
            time_s: (60.0 / bpm.clamp(20.0, 400.0) * self.delay_div.value().beats()).min(2.0),
            feedback: self.delay_feedback.value(),
            mix: self.delay_mix.value(),
            ping_pong: self.delay_ping_pong.value(),
            tone_hz: self.delay_tone.value(),
        }
    }

    /// MIDI note per drum pad (kick, snare, hat).
    pub fn drum_notes(&self) -> [u8; 3] {
        [self.kick_note.value() as u8, self.snare_note.value() as u8, self.hat_note.value() as u8]
    }
}
