//! The mode logic, free of I/O and of the real clock, so every rule is testable.
//!
//! Each tick turns (requests, time, CPU temperature, the mode the hardware reports) into a
//! list of writes. Priority, highest first:
//! 1. emergency temperature or an unreadable temperature → full speed (`Max`)
//! 2. any channel above 0 % → `Manual` with both duties
//! 3. watchdog expired or shutdown → firmware `Auto` at once
//! 4. both at 0 % → `Auto`, after the dwell when coming from `Manual`

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Max = 0,
    Manual = 1,
    Auto = 2,
}

impl Mode {
    pub fn from_raw(v: u32) -> Option<Mode> {
        match v {
            0 => Some(Mode::Max),
            1 => Some(Mode::Manual),
            2 => Some(Mode::Auto),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SetMode(Mode),
    SetPwm(usize, u8),
}

/// Failed temperature reads in a row that count as "temperature lost".
pub const TEMP_FAILURES_LIMIT: u32 = 3;

/// Any `int32` duty from the protocol, clamped to 0–100 %.
pub fn clamp_duty(duty: i32) -> u8 {
    duty.clamp(0, 100) as u8
}

/// Percent to the driver's 0–255 pwm scale: 0 → 0, 100 → 255.
pub fn duty_to_pwm(pct: u8) -> u8 {
    ((pct.min(100) as u32 * 255 + 50) / 100) as u8
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WatchdogTripped,
    EmergencyStart(f64),
    EmergencyEnd(f64),
    TemperatureLost,
    TemperatureBack,
    /// The hardware mode differs from what we last wrote.
    ForeignChange { expected: Mode, found: Mode },
}

#[derive(Debug)]
pub struct Plan {
    pub actions: Vec<Action>,
    target: Mode,
    pwm: [u8; 2],
}

#[derive(Debug)]
pub struct Controller {
    cfg: Config,
    requested: [u8; 2],
    /// Mode we last wrote successfully; None until the first write, or after a failure.
    applied: Option<Mode>,
    /// Duties last written in the current manual session.
    written: Option<[u8; 2]>,
    zero_since: Option<u64>,
    last_rpc: u64,
    force_auto: bool,
    emergency: bool,
    temp_failures: u32,
    pub events: Vec<Event>,
}

impl Controller {
    pub fn new(cfg: Config, now: u64) -> Self {
        Self {
            cfg,
            requested: [0, 0],
            applied: None,
            written: None,
            zero_since: None,
            last_rpc: now,
            force_auto: false,
            emergency: false,
            temp_failures: 0,
            events: Vec::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    #[cfg(test)]
    pub fn applied(&self) -> Option<Mode> {
        self.applied
    }

    pub fn in_emergency(&self) -> bool {
        self.emergency || self.temp_failures >= TEMP_FAILURES_LIMIT
    }

    /// Any RPC from CoolerControl feeds the watchdog.
    pub fn rpc(&mut self, now: u64) {
        self.last_rpc = now;
    }

    pub fn set_duty(&mut self, ch: usize, duty: i32, now: u64) {
        self.rpc(now);
        self.requested[ch] = clamp_duty(duty);
    }

    pub fn reset(&mut self, ch: usize, now: u64) {
        self.set_duty(ch, 0, now);
    }

    /// CoolerControl is shutting the plugin down: firmware auto, no dwell.
    pub fn shutdown(&mut self) {
        self.requested = [0, 0];
        self.force_auto = true;
    }

    /// Records a mode written outside a plan (the startup write of firmware auto).
    pub fn assume_applied(&mut self, m: Mode) {
        self.applied = Some(m);
        self.written = None;
    }

    pub fn tick(&mut self, now: u64, temp: Option<f64>, actual: Option<Mode>) -> Plan {
        // Temperature: a failed read is never "cool".
        match temp {
            Some(t) => {
                if self.temp_failures >= TEMP_FAILURES_LIMIT {
                    self.events.push(Event::TemperatureBack);
                }
                self.temp_failures = 0;
                if !self.emergency && t >= self.cfg.emergency_c {
                    self.emergency = true;
                    self.events.push(Event::EmergencyStart(t));
                } else if self.emergency && t <= self.cfg.emergency_c - self.cfg.hysteresis_c {
                    self.emergency = false;
                    self.events.push(Event::EmergencyEnd(t));
                }
            }
            None => {
                self.temp_failures = self.temp_failures.saturating_add(1);
                if self.temp_failures == TEMP_FAILURES_LIMIT {
                    self.events.push(Event::TemperatureLost);
                }
            }
        }

        // Watchdog: CoolerControl went silent while it had us in manual.
        let silent = now.saturating_sub(self.last_rpc) > self.cfg.watchdog_ms;
        if silent && (self.requested != [0, 0] || self.applied == Some(Mode::Manual)) {
            self.requested = [0, 0];
            self.force_auto = true;
            self.events.push(Event::WatchdogTripped);
        }

        let target = if self.in_emergency() {
            Mode::Max
        } else if self.requested.iter().any(|&d| d > 0) {
            self.force_auto = false;
            Mode::Manual
        } else if self.force_auto || self.applied != Some(Mode::Manual) {
            Mode::Auto
        } else {
            let since = *self.zero_since.get_or_insert(now);
            if now.saturating_sub(since) >= self.cfg.dwell_ms { Mode::Auto } else { Mode::Manual }
        };
        if target != Mode::Manual || self.requested.iter().any(|&d| d > 0) {
            self.zero_since = None;
        }

        if let (Some(expected), Some(found)) = (self.applied, actual) {
            if expected != found {
                self.events.push(Event::ForeignChange { expected, found });
            }
        }

        let pwm = [duty_to_pwm(self.requested[0]), duty_to_pwm(self.requested[1])];
        let mode_ok = self.applied == Some(target) && actual == Some(target);
        let mut actions = Vec::new();
        if !mode_ok {
            actions.push(Action::SetMode(target));
        }
        if target == Mode::Manual {
            for ch in 0..2 {
                if !mode_ok || self.written.map(|w| w[ch]) != Some(pwm[ch]) {
                    actions.push(Action::SetPwm(ch, pwm[ch]));
                }
            }
        }
        Plan { actions, target, pwm }
    }

    /// Every action of the plan was written.
    pub fn commit(&mut self, plan: &Plan) {
        self.applied = Some(plan.target);
        self.written = (plan.target == Mode::Manual).then_some(plan.pwm);
        if plan.target == Mode::Auto {
            self.force_auto = false;
        }
    }

    /// A write failed: nothing of the plan counts as applied, so the next tick writes it all again.
    pub fn failed(&mut self) {
        self.applied = None;
        self.written = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn cfg() -> Config {
        Config::default()
    }

    /// A fake fan driver: applies actions like hp-wmi does, refusing pwm writes outside manual.
    #[derive(Debug)]
    struct Hw {
        mode: Mode,
        pwm: [u8; 2],
        writes: usize,
    }

    impl Hw {
        fn new(mode: Mode) -> Self {
            Self { mode, pwm: [85, 85], writes: 0 }
        }
        fn apply(&mut self, plan: &Plan) -> Result<(), String> {
            for a in &plan.actions {
                self.writes += 1;
                match *a {
                    Action::SetMode(m) => self.mode = m,
                    Action::SetPwm(ch, v) => {
                        if self.mode != Mode::Manual {
                            return Err(format!("pwm write in {:?} (EINVAL)", self.mode));
                        }
                        self.pwm[ch] = v;
                    }
                }
            }
            Ok(())
        }
    }

    /// One control cycle as main.rs runs it.
    fn cycle(c: &mut Controller, hw: &mut Hw, now: u64, temp: Option<f64>) {
        let plan = c.tick(now, temp, Some(hw.mode));
        match hw.apply(&plan) {
            Ok(()) => c.commit(&plan),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn duty_scale_endpoints() {
        assert_eq!(duty_to_pwm(0), 0);
        assert_eq!(duty_to_pwm(100), 255);
        assert_eq!(duty_to_pwm(33), 84);
        assert_eq!(duty_to_pwm(50), 128);
        assert_eq!(clamp_duty(-5), 0);
        assert_eq!(clamp_duty(250), 100);
    }

    proptest! {
        #[test]
        fn duty_scale_every_int32(d in any::<i32>()) {
            let pct = clamp_duty(d);
            prop_assert!(pct <= 100);
            let pwm = duty_to_pwm(pct);
            if d <= 0 { prop_assert_eq!(pwm, 0); }
            if d >= 100 { prop_assert_eq!(pwm, 255); }
            if d > 0 { prop_assert!(pwm > 0); }
        }
    }

    #[test]
    fn mode_both_zero_gives_auto() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Manual);
        cycle(&mut c, &mut hw, 0, Some(40.0));
        assert_eq!(hw.mode, Mode::Auto);
    }

    #[test]
    fn mode_any_nonzero_gives_manual_with_scaled_duties() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 40, 0);
        cycle(&mut c, &mut hw, 0, Some(50.0));
        assert_eq!(hw.mode, Mode::Manual);
        assert_eq!(hw.pwm, [102, 0], "the idle channel is written as 0 and runs at the driver floor");
        // enable before duties
        let mut c = Controller::new(cfg(), 0);
        c.set_duty(1, 60, 0);
        let plan = c.tick(0, Some(50.0), Some(Mode::Auto));
        assert_eq!(plan.actions[0], Action::SetMode(Mode::Manual));
        assert_eq!(&plan.actions[1..], &[Action::SetPwm(0, 0), Action::SetPwm(1, 153)]);
    }

    #[test]
    fn mode_no_pwm_write_in_auto_or_max() {
        // The fake driver panics on a pwm write outside manual; walk through every target.
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        for (t, duty, temp) in [(0, 0, 40.0), (1000, 50, 50.0), (2000, 50, 96.0), (3000, 0, 96.0), (4000, 0, 80.0), (40_000, 0, 80.0)] {
            c.set_duty(0, duty, t);
            cycle(&mut c, &mut hw, t, Some(temp));
        }
        assert_eq!(hw.mode, Mode::Auto);
    }

    #[test]
    fn dwell_holds_manual_then_returns_to_auto() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 50, 0);
        cycle(&mut c, &mut hw, 0, Some(50.0));
        c.set_duty(0, 0, 1000);
        cycle(&mut c, &mut hw, 1000, Some(45.0));
        assert_eq!(hw.mode, Mode::Manual, "dwell starts");
        for t in (2000..30_000).step_by(1000) {
            c.rpc(t);
            cycle(&mut c, &mut hw, t, Some(45.0));
            assert_eq!(hw.mode, Mode::Manual, "still dwelling at {t}");
        }
        c.rpc(31_000);
        cycle(&mut c, &mut hw, 31_000, Some(45.0));
        assert_eq!(hw.mode, Mode::Auto);
    }

    #[test]
    fn dwell_is_cancelled_by_a_nonzero_request() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 50, 0);
        cycle(&mut c, &mut hw, 0, Some(50.0));
        c.set_duty(0, 0, 1000);
        cycle(&mut c, &mut hw, 1000, Some(45.0));
        c.set_duty(1, 10, 20_000);
        cycle(&mut c, &mut hw, 20_000, Some(45.0));
        c.set_duty(1, 0, 21_000);
        for t in (21_000..=50_000).step_by(1000) {
            c.rpc(t);
            cycle(&mut c, &mut hw, t, Some(45.0));
            assert_eq!(hw.mode, Mode::Manual, "at {t}");
        }
        c.rpc(51_000);
        cycle(&mut c, &mut hw, 51_000, Some(45.0));
        assert_eq!(hw.mode, Mode::Auto, "a fresh 30 s dwell from 21 s");
    }

    #[test]
    fn transitions_reset_and_one_channel_zero() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 30, 0);
        c.set_duty(1, 70, 0);
        cycle(&mut c, &mut hw, 0, Some(60.0));
        assert_eq!((hw.mode, hw.pwm), (Mode::Manual, [77, 179]));
        c.reset(1, 1000);
        cycle(&mut c, &mut hw, 1000, Some(60.0));
        assert_eq!((hw.mode, hw.pwm), (Mode::Manual, [77, 0]));
        c.reset(0, 2000);
        c.shutdown();
        cycle(&mut c, &mut hw, 2000, Some(60.0));
        assert_eq!(hw.mode, Mode::Auto, "shutdown skips the dwell");
    }

    #[test]
    fn emergency_overrides_everything_until_limit_minus_hysteresis() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 20, 0);
        cycle(&mut c, &mut hw, 0, Some(94.9));
        assert_eq!(hw.mode, Mode::Manual);
        cycle(&mut c, &mut hw, 1000, Some(95.0));
        assert_eq!(hw.mode, Mode::Max);
        // RPCs, shutdown-free resets and a watchdog expiry do not end it.
        c.reset(0, 2000);
        cycle(&mut c, &mut hw, 2000, Some(93.0));
        assert_eq!(hw.mode, Mode::Max);
        cycle(&mut c, &mut hw, 60_000, Some(90.0));
        assert_eq!(hw.mode, Mode::Max, "watchdog expired, still hot");
        cycle(&mut c, &mut hw, 61_000, Some(85.1));
        assert_eq!(hw.mode, Mode::Max);
        cycle(&mut c, &mut hw, 62_000, Some(85.0));
        assert_eq!(hw.mode, Mode::Auto);
        let starts = c.events.iter().filter(|e| matches!(e, Event::EmergencyStart(_))).count();
        let ends = c.events.iter().filter(|e| matches!(e, Event::EmergencyEnd(_))).count();
        assert_eq!((starts, ends), (1, 1));
    }

    #[test]
    fn emergency_without_rpc() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        cycle(&mut c, &mut hw, 0, Some(50.0));
        cycle(&mut c, &mut hw, 100_000, Some(96.0));
        assert_eq!(hw.mode, Mode::Max);
    }

    #[test]
    fn temp_failure_forces_full_speed_after_three_reads() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 1, 0);
        cycle(&mut c, &mut hw, 0, Some(50.0));
        cycle(&mut c, &mut hw, 1000, None);
        cycle(&mut c, &mut hw, 2000, None);
        assert_eq!(hw.mode, Mode::Manual);
        cycle(&mut c, &mut hw, 3000, None);
        assert_eq!(hw.mode, Mode::Max);
        c.rpc(4000);
        cycle(&mut c, &mut hw, 4000, Some(50.0));
        assert_eq!(hw.mode, Mode::Manual, "back to the curve once the sensor reads again");
    }

    #[test]
    fn watchdog_returns_to_auto_without_dwell() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 50, 0);
        cycle(&mut c, &mut hw, 0, Some(60.0));
        cycle(&mut c, &mut hw, 15_000, Some(60.0));
        assert_eq!(hw.mode, Mode::Manual);
        cycle(&mut c, &mut hw, 15_001, Some(60.0));
        assert_eq!(hw.mode, Mode::Auto);
        assert!(c.events.contains(&Event::WatchdogTripped));
        // Status calls alone do not bring manual back.
        c.rpc(16_000);
        cycle(&mut c, &mut hw, 16_000, Some(60.0));
        assert_eq!(hw.mode, Mode::Auto);
    }

    #[test]
    fn foreign_writer_is_corrected_and_reported() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        cycle(&mut c, &mut hw, 0, Some(40.0));
        hw.mode = Mode::Manual; // e.g. the stock hp device still managed in CoolerControl
        c.rpc(1000);
        cycle(&mut c, &mut hw, 1000, Some(40.0));
        assert_eq!(hw.mode, Mode::Auto);
        assert!(c.events.contains(&Event::ForeignChange { expected: Mode::Auto, found: Mode::Manual }));
    }

    #[test]
    fn write_failure_is_not_applied_and_retried() {
        let mut c = Controller::new(cfg(), 0);
        c.set_duty(0, 50, 0);
        let plan = c.tick(0, Some(50.0), Some(Mode::Auto));
        c.failed();
        assert_eq!(c.applied(), None);
        // Next tick writes everything again, enable first.
        let plan2 = c.tick(1000, Some(50.0), Some(Mode::Manual));
        assert_eq!(plan.actions, plan2.actions);
        assert_eq!(plan2.actions[0], Action::SetMode(Mode::Manual));
    }

    #[test]
    fn churn_no_writes_when_unchanged() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        c.set_duty(0, 50, 0);
        cycle(&mut c, &mut hw, 0, Some(60.0));
        let before = hw.writes;
        for t in (1000..20_000).step_by(1000) {
            c.set_duty(0, 50, t);
            cycle(&mut c, &mut hw, t, Some(60.0));
        }
        assert_eq!(hw.writes, before);
    }

    #[test]
    fn churn_alternating_requests_switch_to_auto_at_most_once_per_dwell() {
        let mut c = Controller::new(cfg(), 0);
        let mut hw = Hw::new(Mode::Auto);
        let mut to_auto = Vec::new();
        let mut last = hw.mode;
        for i in 0..600u64 {
            let t = i * 1000;
            c.set_duty(0, (i % 2) as i32, t);
            cycle(&mut c, &mut hw, t, Some(50.0));
            if last == Mode::Manual && hw.mode == Mode::Auto {
                to_auto.push(t);
            }
            last = hw.mode;
        }
        assert!(to_auto.is_empty(), "a request every other second never lets the dwell run out: {to_auto:?}");
    }

    #[derive(Debug, Clone)]
    enum Op {
        Duty(usize, i32),
        Reset(usize),
        Status,
        Wait(u64),
        Temp(Option<f64>),
        Foreign(Mode),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0usize..2, -10i32..120).prop_map(|(c, d)| Op::Duty(c, d)),
            (0usize..2).prop_map(Op::Reset),
            Just(Op::Status),
            (0u64..40_000).prop_map(Op::Wait),
            prop_oneof![Just(None), (20.0f64..110.0).prop_map(Some)].prop_map(Op::Temp),
            prop_oneof![Just(Mode::Max), Just(Mode::Manual), Just(Mode::Auto)].prop_map(Op::Foreign),
        ]
    }

    proptest! {
        /// ISC-22: the fans are never left in manual mode once CoolerControl has been silent
        /// longer than the watchdog, and pwm is never written outside manual (the fake driver
        /// refuses it, like hp-wmi).
        #[test]
        fn proptest_never_manual_without_a_live_controller(ops in proptest::collection::vec(op(), 1..200)) {
            let cfg = cfg();
            let mut c = Controller::new(cfg.clone(), 0);
            let mut hw = Hw::new(Mode::Auto);
            let mut now = 0u64;
            let mut last_rpc = 0u64;
            let mut temp = Some(50.0);
            for o in ops {
                match o {
                    Op::Duty(ch, d) => { c.set_duty(ch, d, now); last_rpc = now; }
                    Op::Reset(ch) => { c.reset(ch, now); last_rpc = now; }
                    Op::Status => { c.rpc(now); last_rpc = now; }
                    Op::Wait(ms) => now += ms,
                    Op::Temp(t) => temp = t,
                    Op::Foreign(m) => hw.mode = m,
                }
                let plan = c.tick(now, temp, Some(hw.mode));
                prop_assert!(hw.apply(&plan).is_ok());
                c.commit(&plan);
                if now - last_rpc > cfg.watchdog_ms {
                    prop_assert_ne!(hw.mode, Mode::Manual, "manual {} ms after the last RPC", now - last_rpc);
                }
                if temp.map_or(false, |t| t >= cfg.emergency_c) {
                    prop_assert_eq!(hw.mode, Mode::Max);
                }
            }
        }
    }
}
