//! Runs the controller against the real (or fake) sysfs: one `cycle` reads the hardware,
//! asks the controller for a plan, writes it, and reports what happened.

use crate::config::Config;
use crate::control::{Action, Controller, Event, Mode};
use crate::hw::{self, HpFans};
use log::{debug, error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Path the panic hook writes firmware auto to. Updated when the device is rediscovered.
pub static PANIC_ENABLE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub struct Engine {
    root: PathBuf,
    fans: Option<HpFans>,
    cpu_temp: Option<PathBuf>,
    pub ctrl: Controller,
    started: Instant,
    rpm: [Option<u32>; 2],
    pwm: [Option<u32>; 2],
    foreign_warned_at: Option<u64>,
    write_error_logged: bool,
}

impl Engine {
    pub fn new(root: &Path, fans: HpFans, cpu_temp: PathBuf, cfg: Config) -> Self {
        set_panic_path(&fans);
        Self {
            root: root.to_path_buf(),
            fans: Some(fans),
            cpu_temp: Some(cpu_temp),
            ctrl: Controller::new(cfg, 0),
            started: Instant::now(),
            rpm: [None, None],
            pwm: [None, None],
            foreign_warned_at: None,
            write_error_logged: false,
        }
    }

    pub fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Firmware auto, written before the plugin serves anything.
    pub fn start(&mut self) -> anyhow::Result<()> {
        let fans = self.fans.as_ref().expect("fans present at start");
        fans.set_mode(Mode::Auto)?;
        self.ctrl.assume_applied(Mode::Auto);
        info!("firmware auto set at start ({})", fans.dir().display());
        Ok(())
    }

    /// Last fan readings for Status: RPM and the driver's pwm reading as a percent.
    pub fn readings(&self) -> [(Option<u32>, Option<f64>); 2] {
        [0, 1].map(|ch| (self.rpm[ch], self.pwm[ch].map(|p| (p as f64 * 100.0 / 255.0).round())))
    }

    fn rediscover(&mut self) {
        match hw::find_hp(&self.root) {
            Ok(f) => {
                info!("hp fan device found at {}", f.dir().display());
                set_panic_path(&f);
                self.fans = Some(f);
            }
            Err(e) => debug!("hp fan device still missing: {e}"),
        }
        if self.cpu_temp.as_ref().is_none_or(|p| !p.exists()) {
            self.cpu_temp = hw::find_cpu_temp(&self.root);
        }
    }

    pub fn cycle(&mut self) {
        let now = self.now();
        if self.fans.is_none() {
            self.rediscover();
        }
        let temp = self.cpu_temp.as_ref().and_then(|p| match hw::read_celsius(p) {
            Ok(t) => Some(t),
            Err(e) => {
                debug!("CPU temperature read failed: {e}");
                None
            }
        });
        if temp.is_none() {
            self.cpu_temp = None;
            self.rediscover();
        }
        let actual = match &self.fans {
            Some(f) => match f.read_mode() {
                Ok(m) => Some(m),
                Err(e) => {
                    warn!("reading fan mode failed ({e}); looking for the device again");
                    self.fans = None;
                    self.rediscover();
                    None
                }
            },
            None => None,
        };
        if let Some(f) = &self.fans {
            for ch in 0..2 {
                self.rpm[ch] = f.read_rpm(ch).ok();
                self.pwm[ch] = f.read_pwm(ch).ok();
            }
        }

        let plan = self.ctrl.tick(now, temp, actual);
        self.report_events(now);
        if plan.actions.is_empty() {
            return;
        }
        let Some(fans) = &self.fans else {
            self.ctrl.failed();
            return;
        };
        let mut result = Ok(());
        for a in &plan.actions {
            result = match *a {
                Action::SetMode(m) => fans.set_mode(m),
                Action::SetPwm(ch, v) => fans.set_pwm(ch, v),
            };
            debug!("{a:?} -> {}", if result.is_ok() { "ok" } else { "failed" });
            if result.is_err() {
                break;
            }
        }
        match result {
            Ok(()) => {
                if self.write_error_logged {
                    info!("fan writes work again");
                    self.write_error_logged = false;
                }
                self.ctrl.commit(&plan);
            }
            Err(e) => {
                if !self.write_error_logged {
                    error!("fan write failed, retrying every tick: {e:#}");
                    self.write_error_logged = true;
                }
                self.ctrl.failed();
            }
        }
    }

    fn report_events(&mut self, now: u64) {
        for ev in std::mem::take(&mut self.ctrl.events) {
            match ev {
                Event::WatchdogTripped => warn!(
                    "no request from CoolerControl for {} s: fans handed back to firmware auto",
                    self.ctrl.config().watchdog_ms / 1000
                ),
                Event::EmergencyStart(t) => error!("CPU at {t:.1} °C: fans at full speed until it falls below the limit"),
                Event::EmergencyEnd(t) => warn!("CPU back at {t:.1} °C: emergency over, fans follow the curves again"),
                Event::TemperatureLost => error!("CPU temperature unreadable: fans at full speed until it reads again"),
                Event::TemperatureBack => warn!("CPU temperature readable again"),
                Event::ForeignChange { expected, found } => {
                    if self.foreign_warned_at.is_none_or(|t| now.saturating_sub(t) > 60_000) {
                        warn!(
                            "fan mode changed outside the plugin ({expected:?} -> {found:?}); restoring. \
                             If this repeats, disable the stock 'hp' device in CoolerControl."
                        );
                        self.foreign_warned_at = Some(now);
                    } else {
                        debug!("fan mode changed outside the plugin ({expected:?} -> {found:?}); restoring");
                    }
                }
            }
        }
    }

    /// Exit path: firmware auto, whatever the controller thinks.
    pub fn stop(&mut self) {
        self.ctrl.shutdown();
        self.cycle();
        if let Some(f) = &self.fans {
            match f.set_mode(Mode::Auto) {
                Ok(()) => info!("fans handed back to firmware auto"),
                Err(e) => error!("could not hand the fans back to firmware auto: {e:#}"),
            }
        }
    }
}

fn set_panic_path(fans: &HpFans) {
    if let Ok(mut p) = PANIC_ENABLE_PATH.lock() {
        *p = Some(fans.enable_path());
    }
}

/// A panic anywhere hands the fans to the firmware and, in the real service, ends the process,
/// so a half-dead plugin never keeps answering CoolerControl.
pub fn install_panic_hook(exit: bool) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = PANIC_ENABLE_PATH.lock().map(|p| p.clone()).unwrap_or_else(|p| p.into_inner().clone());
        if let Some(p) = path {
            let _ = hw::force_auto(&p);
        }
        default(info);
        if exit {
            std::process::abort();
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::fake::FakeSys;

    static LOCK: Mutex<()> = Mutex::new(());

    fn engine(s: &FakeSys) -> Engine {
        let fans = hw::find_hp(s.root()).unwrap();
        let temp = hw::find_cpu_temp(s.root()).unwrap();
        Engine::new(s.root(), fans, temp, Config::default())
    }

    #[test]
    fn process_start_writes_auto_before_anything_else() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "1");
        let mut e = engine(&s);
        e.start().unwrap();
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "2");
    }

    #[test]
    fn engine_drives_the_fake_sysfs() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        let mut e = engine(&s);
        e.start().unwrap();
        e.ctrl.set_duty(0, 40, e.now());
        e.cycle();
        assert_eq!((s.get("hwmon8", "pwm1_enable"), s.get("hwmon8", "pwm1"), s.get("hwmon8", "pwm2")), ("1".into(), "102".into(), "0".into()));
        assert_eq!(s.get("hwmon3", "pwm1_enable"), "1", "decoy untouched");
        e.stop();
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "2");
    }

    #[test]
    fn write_failure_retried_on_next_cycle() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        let mut e = engine(&s);
        e.start().unwrap();
        // pwm1 vanishes: the duty write fails, nothing counts as applied.
        std::fs::remove_file(s.path("hwmon8", "pwm1")).unwrap();
        e.ctrl.set_duty(0, 40, e.now());
        e.cycle();
        assert_eq!(e.ctrl.applied(), None);
        std::fs::write(s.path("hwmon8", "pwm1"), "85").unwrap();
        e.cycle();
        assert_eq!(s.get("hwmon8", "pwm1"), "102");
        assert_eq!(e.ctrl.applied(), Some(Mode::Manual));
    }

    #[test]
    fn rediscover_after_the_device_moves() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        let mut e = engine(&s);
        e.start().unwrap();
        std::fs::rename(s.root().join("class/hwmon/hwmon8"), s.root().join("class/hwmon/hwmon11")).unwrap();
        e.ctrl.set_duty(1, 50, e.now());
        e.cycle(); // read fails, device found again under hwmon11
        e.cycle();
        assert_eq!(s.get("hwmon11", "pwm1_enable"), "1");
        assert_eq!(s.get("hwmon11", "pwm2"), "128");
        assert_eq!(PANIC_ENABLE_PATH.lock().unwrap().as_ref().unwrap(), &s.path("hwmon11", "pwm1_enable"));
    }

    #[test]
    fn temp_failure_in_engine_forces_full_speed() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        let mut e = engine(&s);
        e.start().unwrap();
        std::fs::remove_file(s.path("hwmon5", "temp1_input")).unwrap();
        for _ in 0..3 {
            e.cycle();
        }
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "0");
    }

    #[test]
    fn panic_hook_hands_fans_to_firmware() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = FakeSys::new();
        let _e = engine(&s); // registers the enable path
        std::fs::write(s.path("hwmon8", "pwm1_enable"), "1").unwrap();
        install_panic_hook(false);
        let r = std::thread::spawn(|| panic!("injected")).join();
        assert!(r.is_err());
        assert_eq!(s.get("hwmon8", "pwm1_enable"), "2");
    }
}
