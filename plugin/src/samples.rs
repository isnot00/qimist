//! Custom drum samples. WAV files live in one folder (`~/Music/Qimist/Samples`)
//! so each pad can simply step through them — no file dialog needed, which is
//! fragile inside plugin hosts. Files are decoded off the audio thread and
//! handed over through `Shared`; a cache keeps every loaded sample alive so the
//! audio thread never frees memory when it swaps one out.

use crate::params::VoxParams;
use crate::shared::Shared;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use qimist_dsp::drums::load_wav;

pub fn samples_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Music").join("Qimist").join("Samples")
}

/// WAV files in the samples folder, sorted by name.
pub fn list() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(samples_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

pub fn display_name(path: &str) -> String {
    if path.is_empty() {
        return "BUILT-IN".into();
    }
    Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

fn load_pad(shared: &Shared, pad: usize, path: &str) {
    let sr = shared.sample_rate.load(Relaxed);
    let (sample, status) = if path.is_empty() {
        (None, String::new())
    } else {
        match load_wav(Path::new(path), sr) {
            Ok(data) => {
                let s = Arc::new(data);
                shared.sample_cache.lock().push(s.clone());
                (Some(s), String::new())
            }
            Err(e) => (None, format!("can't load: {e}")),
        }
    };
    shared.drum_sample_slots.lock()[pad] = sample;
    shared.sample_status.lock()[pad] = status;
    shared.sample_version.fetch_add(1, Relaxed);
}

/// (Re)load every pad from the paths saved in the project.
pub fn load_all(params: &VoxParams, shared: &Shared) {
    let paths = params.drum_samples.read().clone();
    for (pad, path) in paths.iter().enumerate() {
        load_pad(shared, pad, path);
    }
}

/// Choose a sample (or `None` for the built-in sound) for one pad.
pub fn set(params: &VoxParams, shared: &Shared, pad: usize, path: Option<&Path>) {
    let p = path.map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    params.drum_samples.write()[pad] = p.clone();
    load_pad(shared, pad, &p);
}
