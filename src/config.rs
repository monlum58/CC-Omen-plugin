//! Plugin settings from `config.json`, clamped so no value can weaken the safety paths.

use serde::Deserialize;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

/// Bounds every setting is clamped into. Documented in the README.
pub const DWELL_S: (u64, u64) = (0, 600);
pub const WATCHDOG_S: (u64, u64) = (3, 60);
pub const EMERGENCY_C: (f64, f64) = (70.0, 100.0);
pub const HYSTERESIS_C: (f64, f64) = (3.0, 20.0);

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Both channels must stay at 0 % this long before the fans go back to firmware auto.
    pub dwell_ms: u64,
    /// Manual mode without any RPC for this long falls back to firmware auto.
    pub watchdog_ms: u64,
    /// CPU temperature that forces full speed.
    pub emergency_c: f64,
    /// Full speed ends only at `emergency_c - hysteresis_c`.
    pub hysteresis_c: f64,
    /// Control loop period.
    pub tick_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { dwell_ms: 30_000, watchdog_ms: 15_000, emergency_c: 95.0, hysteresis_c: 10.0, tick_ms: 1_000 }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct Raw {
    dwell_seconds: Option<u64>,
    watchdog_seconds: Option<u64>,
    emergency_celsius: Option<f64>,
    emergency_hysteresis_celsius: Option<f64>,
}

/// Loads the config, returning it together with one human-readable note per thing that
/// was ignored or clamped. A missing file is not worth a note.
pub fn load(path: &Path, owner_uid: u32) -> (Config, Vec<String>) {
    let mut notes = Vec::new();
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return (Config::default(), notes),
    };
    let ft = meta.file_type();
    if ft.is_symlink() || !ft.is_file() || ft.is_socket() {
        notes.push(format!("{}: not a regular file, ignored; using defaults", path.display()));
        return (Config::default(), notes);
    }
    if meta.uid() != owner_uid {
        notes.push(format!("{}: owned by uid {}, not {owner_uid}; ignored, using defaults", path.display(), meta.uid()));
        return (Config::default(), notes);
    }
    if meta.mode() & 0o022 != 0 {
        notes.push(format!("{}: writable by group or others; ignored, using defaults", path.display()));
        return (Config::default(), notes);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            notes.push(format!("{}: unreadable ({e}); using defaults", path.display()));
            return (Config::default(), notes);
        }
    };
    match parse(&text, &mut notes) {
        Some(c) => (c, notes),
        None => (Config::default(), notes),
    }
}

/// Parses and clamps. Returns None (and a note) for malformed JSON or unknown keys.
pub fn parse(text: &str, notes: &mut Vec<String>) -> Option<Config> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v @ serde_json::Value::Object(_)) => v,
        Ok(_) => {
            notes.push("config.json rejected (not a JSON object); using defaults".to_string());
            return None;
        }
        Err(e) => {
            notes.push(format!("config.json rejected ({e}); using defaults"));
            return None;
        }
    };
    let raw: Raw = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            notes.push(format!("config.json rejected ({e}); using defaults"));
            return None;
        }
    };
    let d = Config::default();
    let mut clamp_u = |name: &str, v: Option<u64>, (lo, hi): (u64, u64), def: u64| match v {
        None => def,
        Some(x) if x < lo || x > hi => {
            let c = x.clamp(lo, hi);
            notes.push(format!("{name} = {x} is outside {lo}..={hi}; using {c}"));
            c
        }
        Some(x) => x,
    };
    let dwell = clamp_u("dwell_seconds", raw.dwell_seconds, DWELL_S, d.dwell_ms / 1000);
    let watchdog = clamp_u("watchdog_seconds", raw.watchdog_seconds, WATCHDOG_S, d.watchdog_ms / 1000);
    let mut clamp_f = |name: &str, v: Option<f64>, (lo, hi): (f64, f64), def: f64| match v {
        None => def,
        Some(x) if !x.is_finite() || x < lo || x > hi => {
            let c = if x.is_finite() { x.clamp(lo, hi) } else { def };
            notes.push(format!("{name} = {x} is outside {lo}..={hi}; using {c}"));
            c
        }
        Some(x) => x,
    };
    let emergency = clamp_f("emergency_celsius", raw.emergency_celsius, EMERGENCY_C, d.emergency_c);
    let hysteresis = clamp_f("emergency_hysteresis_celsius", raw.emergency_hysteresis_celsius, HYSTERESIS_C, d.hysteresis_c);
    Some(Config { dwell_ms: dwell * 1000, watchdog_ms: watchdog * 1000, emergency_c: emergency, hysteresis_c: hysteresis, tick_ms: d.tick_ms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    fn write(dir: &Path, text: &str, mode: u32) -> std::path::PathBuf {
        let p = dir.join("config.json");
        std::fs::write(&p, text).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn config_missing_gives_defaults_without_notes() {
        let d = tempfile::tempdir().unwrap();
        let (c, n) = load(&d.path().join("config.json"), uid());
        assert_eq!(c, Config::default());
        assert!(n.is_empty());
    }

    #[test]
    fn config_valid_values_are_used() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), r#"{"dwell_seconds":45,"watchdog_seconds":10,"emergency_celsius":90,"emergency_hysteresis_celsius":5}"#, 0o644);
        let (c, n) = load(&p, uid());
        assert!(n.is_empty(), "{n:?}");
        assert_eq!((c.dwell_ms, c.watchdog_ms, c.emergency_c, c.hysteresis_c), (45_000, 10_000, 90.0, 5.0));
    }

    #[test]
    fn config_bad_values_are_clamped_with_a_note() {
        let table = [
            (r#"{"emergency_celsius":200}"#, "emergency", 100.0),
            (r#"{"emergency_celsius":-5}"#, "emergency", 70.0),
            (r#"{"watchdog_seconds":0}"#, "watchdog", 3.0),
            (r#"{"watchdog_seconds":18446744073709551615}"#, "watchdog", 60.0),
            (r#"{"dwell_seconds":100000}"#, "dwell", 600.0),
            (r#"{"emergency_hysteresis_celsius":0}"#, "hysteresis", 3.0),
        ];
        for (text, field, want) in table {
            let mut notes = Vec::new();
            let c = parse(text, &mut notes).unwrap();
            let got = match field {
                "emergency" => c.emergency_c,
                "watchdog" => (c.watchdog_ms / 1000) as f64,
                "dwell" => (c.dwell_ms / 1000) as f64,
                _ => c.hysteresis_c,
            };
            assert_eq!(got, want, "{text}");
            assert_eq!(notes.len(), 1, "{text}: {notes:?}");
        }
    }

    #[test]
    fn config_malformed_or_unknown_keys_fall_back_to_defaults() {
        for text in ["{not json", r#"{"emergency_celsius":null,"extra":1}"#, r#"{"disable_safety":true}"#, r#"{"emergency_celsius":"95"}"#, "[]"] {
            let mut notes = Vec::new();
            assert!(parse(text, &mut notes).is_none(), "{text}");
            assert_eq!(notes.len(), 1);
        }
        let mut notes = Vec::new();
        // null on a known key is just "not set".
        assert_eq!(parse(r#"{"emergency_celsius":null}"#, &mut notes), Some(Config::default()));
    }

    #[test]
    fn config_unsafe_files_are_ignored() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), r#"{"emergency_celsius":70}"#, 0o666);
        let (c, n) = load(&p, uid());
        assert_eq!(c, Config::default());
        assert_eq!(n.len(), 1);

        let p = write(d.path(), r#"{"emergency_celsius":70}"#, 0o644);
        let (c, n) = load(&p, uid().wrapping_add(1));
        assert_eq!(c, Config::default());
        assert_eq!(n.len(), 1);

        let link = d.path().join("link.json");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        let (c, n) = load(&link, uid());
        assert_eq!(c, Config::default());
        assert_eq!(n.len(), 1);
    }
}
