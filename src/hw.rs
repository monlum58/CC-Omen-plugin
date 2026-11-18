//! Access to the hp-wmi hwmon files. Every write goes through `HpFans::write_attr`, which only
//! opens one of three fixed attribute names inside the hp hwmon directory, never creates a file,
//! and never builds a path from request data.

use crate::control::Mode;
use anyhow::{Context, Result, anyhow, bail};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SUPPORTED_BOARD: &str = "8BCA";
const REQUIRED: [&str; 5] = ["pwm1_enable", "pwm1", "pwm2", "fan1_input", "fan2_input"];
const PWM: [&str; 2] = ["pwm1", "pwm2"];
const FAN: [&str; 2] = ["fan1_input", "fan2_input"];

#[derive(Debug, Clone)]
pub struct HpFans {
    dir: PathBuf,
}

fn hwmon_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("class/hwmon"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn hwmon_name(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("name")).ok().map(|s| s.trim().to_string())
}

/// Finds the hwmon device named exactly `hp` that has every fan attribute. Found by name,
/// never by index, so another device with a `pwm1_enable` is never touched.
pub fn find_hp(root: &Path) -> Result<HpFans> {
    for dir in hwmon_dirs(root) {
        if hwmon_name(&dir).as_deref() != Some("hp") {
            continue;
        }
        if let Some(missing) = REQUIRED.iter().find(|a| !dir.join(a).exists()) {
            bail!("hwmon device 'hp' at {} has no {missing}: this hp-wmi driver has no separate fan control", dir.display());
        }
        return Ok(HpFans { dir });
    }
    Err(anyhow!("no hwmon device named 'hp' with pwm1_enable, pwm1 and pwm2 (needs the in-tree hp-wmi driver of Linux 7.3 or newer)"))
}

/// The CPU temperature input: k10temp (AMD) or coretemp (Intel), found by name.
pub fn find_cpu_temp(root: &Path) -> Option<PathBuf> {
    hwmon_dirs(root).into_iter().find_map(|dir| match hwmon_name(&dir).as_deref() {
        Some("k10temp") | Some("coretemp") => Some(dir.join("temp1_input")).filter(|p| p.exists()),
        _ => None,
    })
}

pub fn board_name(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("class/dmi/id/board_name")).ok().map(|s| s.trim().to_string())
}

/// Reads a millidegree temperature file. Anything unreadable or implausible is an error,
/// which the controller counts as a failed read, never as "cool".
pub fn read_celsius(path: &Path) -> Result<f64> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let milli: i64 = raw.trim().parse().with_context(|| format!("parsing {:?}", raw.trim()))?;
    let c = milli as f64 / 1000.0;
    if !(-40.0..=150.0).contains(&c) {
        bail!("implausible CPU temperature {c} °C");
    }
    Ok(c)
}

impl HpFans {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn enable_path(&self) -> PathBuf {
        self.dir.join("pwm1_enable")
    }

    fn read_u32(&self, attr: &str) -> Result<u32> {
        let p = self.dir.join(attr);
        let raw = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        raw.trim().parse().with_context(|| format!("parsing {} = {:?}", p.display(), raw.trim()))
    }

    fn write_attr(&self, attr: &'static str, value: u32) -> Result<()> {
        debug_assert!(attr == "pwm1_enable" || PWM.contains(&attr));
        let p = self.dir.join(attr);
        // No create: a missing file means the device went away, never something to make.
        let mut f = OpenOptions::new().write(true).truncate(true).open(&p).with_context(|| format!("opening {}", p.display()))?;
        f.write_all(value.to_string().as_bytes()).with_context(|| format!("writing {value} to {}", p.display()))
    }

    pub fn read_mode(&self) -> Result<Mode> {
        let v = self.read_u32("pwm1_enable")?;
        Mode::from_raw(v).ok_or_else(|| anyhow!("unknown pwm1_enable value {v}"))
    }

    pub fn set_mode(&self, m: Mode) -> Result<()> {
        self.write_attr("pwm1_enable", m as u32)
    }

    pub fn set_pwm(&self, ch: usize, pwm: u8) -> Result<()> {
        self.write_attr(PWM[ch], pwm as u32)
    }

    pub fn read_pwm(&self, ch: usize) -> Result<u32> {
        self.read_u32(PWM[ch])
    }

    pub fn read_rpm(&self, ch: usize) -> Result<u32> {
        self.read_u32(FAN[ch])
    }
}

/// Writes firmware auto to an `pwm1_enable` path directly. Used by the panic hook and on exit,
/// where nothing else may be trusted.
pub fn force_auto(enable_path: &Path) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).truncate(true).open(enable_path)?;
    f.write_all(b"2")
}

#[cfg(test)]
pub mod fake {
    //! A fake /sys tree for tests.
    use std::path::{Path, PathBuf};

    pub struct FakeSys {
        pub dir: tempfile::TempDir,
    }

    impl FakeSys {
        /// hwmon3 is a decoy with its own pwm1_enable, hwmon5 is k10temp, hwmon8 is hp.
        pub fn new() -> Self {
            let s = Self { dir: tempfile::tempdir().unwrap() };
            s.add("hwmon3", "amdgpu", &[("pwm1_enable", "1"), ("pwm1", "100")]);
            s.add("hwmon5", "k10temp", &[("temp1_input", "45000")]);
            s.add("hwmon8", "hp", &[("pwm1_enable", "1"), ("pwm1", "85"), ("pwm2", "85"), ("fan1_input", "2000"), ("fan2_input", "2000")]);
            std::fs::create_dir_all(s.root().join("class/dmi/id")).unwrap();
            std::fs::write(s.root().join("class/dmi/id/board_name"), "8BCA\n").unwrap();
            s
        }
        pub fn root(&self) -> &Path {
            self.dir.path()
        }
        pub fn add(&self, hw: &str, name: &str, files: &[(&str, &str)]) {
            let d = self.root().join("class/hwmon").join(hw);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
            for (f, v) in files {
                std::fs::write(d.join(f), format!("{v}\n")).unwrap();
            }
        }
        pub fn path(&self, hw: &str, f: &str) -> PathBuf {
            self.root().join("class/hwmon").join(hw).join(f)
        }
        pub fn get(&self, hw: &str, f: &str) -> String {
            std::fs::read_to_string(self.path(hw, f)).unwrap().trim().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeSys;
    use super::*;

    #[test]
    fn discovery_finds_hp_by_name_and_ignores_decoys() {
        let s = FakeSys::new();
        let hp = find_hp(s.root()).unwrap();
        assert!(hp.dir().ends_with("hwmon8"));
        hp.set_mode(Mode::Auto).unwrap();
        hp.set_pwm(0, 7).unwrap();
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "2");
        assert_eq!(s.get("hwmon8", "pwm1"), "7");
        // The decoy with its own pwm1_enable is untouched.
        assert_eq!(s.get("hwmon3", "pwm1_enable"), "1");
        assert_eq!(s.get("hwmon3", "pwm1"), "100");
        assert!(find_cpu_temp(s.root()).unwrap().ends_with("hwmon5/temp1_input"));
        assert_eq!(board_name(s.root()).as_deref(), Some(SUPPORTED_BOARD));
    }

    #[test]
    fn discovery_refuses_without_the_fan_interface() {
        let empty = tempfile::tempdir().unwrap();
        assert!(find_hp(empty.path()).is_err());
        let s = FakeSys::new();
        std::fs::remove_file(s.path("hwmon8", "pwm2")).unwrap();
        let e = find_hp(s.root()).unwrap_err().to_string();
        assert!(e.contains("pwm2"), "{e}");
    }

    #[test]
    fn writes_never_create_files() {
        let s = FakeSys::new();
        let hp = find_hp(s.root()).unwrap();
        std::fs::remove_file(s.path("hwmon8", "pwm1")).unwrap();
        assert!(hp.set_pwm(0, 50).is_err());
        assert!(!s.path("hwmon8", "pwm1").exists());
    }

    #[test]
    fn shorter_values_replace_longer_ones() {
        let s = FakeSys::new();
        let hp = find_hp(s.root()).unwrap();
        hp.set_pwm(1, 255).unwrap();
        hp.set_pwm(1, 0).unwrap();
        assert_eq!(hp.read_pwm(1).unwrap(), 0);
    }

    #[test]
    fn temperature_reads_reject_garbage() {
        let s = FakeSys::new();
        let p = find_cpu_temp(s.root()).unwrap();
        assert_eq!(read_celsius(&p).unwrap(), 45.0);
        for bad in ["", "abc", "999999", "-100000"] {
            std::fs::write(&p, bad).unwrap();
            assert!(read_celsius(&p).is_err(), "{bad:?}");
        }
        std::fs::remove_file(&p).unwrap();
        assert!(read_celsius(&p).is_err());
    }
}
