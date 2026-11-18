---
name: "CC-Omen-plugin"
task: "CoolerControl device plugin that lets fan curves stop the HP OMEN laptop fans"
slug: 20261118-154500_cc-omen-plugin
project: CC-Omen-plugin
phase: climbing
progress: 36/47
started: 2026-11-18T15:45:00+01:00
updated: 2026-11-18T16:35:00+01:00
principal_stated_goal: 'CoolerControl is great, but it cannot stop the fans completely. Convince CoolerControl to stop the fans if the system temperature is low, while the fans still react early enough to keep the hardware cool.'
principal_stated_goal_source: conversation 2026-11-18
principal_stated_goal_signal: 3
principal_stated_goal_locked: 2026-11-18T15:45:00+01:00
context_sufficient: true
interview_invoked: false
absence_review: Opus@2026-11-18T15:58:00+01:00
---

## Ideal State

### L3 · One sentence

On an HP OMEN 16 (board 8BCA) under Linux, CoolerControl alone runs both fans: silent at 0 RPM while the machine is cool, spinning up exactly where the user's own curves say, and never leaving the hardware without a working fan controller, with Secure Boot on.

### L2 · Summary

CC-Omen-plugin is a small CoolerControl device plugin that makes 0 % on a curve mean "hand the fans back to the firmware, which stops them", and anything above 0 % mean "run the fans manually at this speed".

- For the person at the desk: the laptop is silent at idle and quiet at light load, and gets loud only when their curve says so, sooner than the firmware's own auto mode.
- For the hardware: if CoolerControl, the plugin or the curve fails, the fans fall back to firmware control or full speed, never to a low fixed speed with nobody watching.
- For the system: no kernel module, no Python, one Rust binary in CoolerControl's plugin folder, so distribution updates and Secure Boot don't touch it.
- For maintenance: one install command, and one uninstall command that leaves the machine exactly as the stock driver had it.

### L1 · Full articulation

Since Linux 7.3's in-tree `hp-wmi`, the OMEN 16-xf0xxx (board 8BCA) exposes its CPU fan as `pwm1` and its GPU fan as `pwm2`, with one shared mode switch `pwm1_enable` (0 = full speed, 1 = manual, 2 = firmware auto). In manual mode the driver clamps every request to the lowest step of the BIOS fan table, about 2000 RPM on this model, so no manual value can stop a fan. In firmware auto mode, with "Fan Always On" off in the BIOS, the firmware stops both fans while the machine is cool, but starts them late under load: temperatures climb well past what the vendor's Windows software allows.

The plugin presents itself to CoolerControl as one device, "HP OMEN fans", with two speed channels (CPU fan, GPU fan) at 0–100 %. CoolerControl drives them with ordinary curves, for example from the CPU temperature (k10temp) and the GPU hotspot temperature. When both channels are asked for 0 %, the plugin sets firmware auto and the firmware stops the fans. When either channel is asked for more than 0 %, the plugin switches to manual and writes both speeds; a channel asked for 0 % in manual mode runs at the driver's floor. Switching back to auto waits for a short dwell time so the fans don't flap at the threshold.

The plugin is the only writer to the fans: the stock `hp` hwmon device stays unmanaged in CoolerControl. On every way out (CoolerControl stops the plugin, the plugin crashes or is killed, the gRPC client goes silent, the plugin is uninstalled) the fans go back to firmware auto. A temperature guard inside the plugin forces full speed if the CPU passes an emergency limit, whatever CoolerControl asks. After suspend and resume the requested state is applied again.

It installs with one command into `/var/lib/coolercontrol/plugins/cc_omen/`, carries the SELinux `bin_t` label it needs on Fedora-based systems, and refuses to start on a machine without this driver interface.

## Problem

CoolerControl drives the `hp` hwmon fans in manual mode only, so the driver's minimum-RPM clamp makes "0 %" impossible. Firmware auto mode stops the fans but cools too late. Earlier workarounds patched and rebuilt `hp-wmi`; the in-tree driver replaces them, and manual-mode tools now hit the same floor.

## Vision

Open the lid, read mail, write code: the laptop is silent. Start a build; at the curve's first step the fan comes in, quietly, well before anything gets hot. The build ends, the CPU cools, half a minute later the fans stop. Nobody thinks about fans again, and the next distribution update doesn't change that.

## Out of Scope

- **No kernel module.** No patched `hp-wmi`, no DKMS or akmods. Removing the minimum-RPM clamp in the driver is a separate upstream request.
- **No own fan curves or UI.** Curves, temperature sources and graphs stay in CoolerControl.
- **No support promise for other laptops.** Board 8BCA with the in-tree `hp-wmi` interface is the tested target; other boards with the same sysfs interface may work.

## Constraints

- Driver interface (upstream `drivers/platform/x86/hp/hp-wmi.c`, read 2026-11-18): `pwm1_enable` ∈ {0 max, 1 manual, 2 auto}; `pwm1`/`pwm2` are writable only in manual (`-EINVAL` otherwise) and are clamped to `[min_rpm, max_rpm]` of the BIOS fan table; the driver runs its own keep-alive in manual and max mode.
- CoolerControl 5.1.x device-service plugin contract: `manifest.toml` in `/var/lib/coolercontrol/plugins/<id>/`, gRPC over a Unix socket, RPCs `ListDevices`, `InitializeDevice`, `Status`, `FixedDuty`, `ResetChannel`, `EnableManualFanControl`, `Shutdown`; `privileged = true` runs it as root.
- SELinux enforcing: the executable needs `bin_t` (`semanage fcontext` + `restorecon`), or systemd fails with `203/EXEC`.
- Rust, starting from CoolerControl's official `device-service-template` (GPL-3.0); helper scripts in TypeScript on bun.
- Installing needs root; tests, sysfs reads and logs run as a normal user.
- The repository is meant to become public: no personal names, hosts, paths or anecdotes in tracked files, issues or commit messages.

## Features

### F0 · Project
Why: the plugin builds, versions and proves itself from one command.

- [x] ISC-1: `cargo test` and `cargo build --release` pass on a clean checkout. — ✓ cargo test 33+6 green; cargo build --release --locked ok
- [x] ISC-2: A git pre-commit hook raises the build number in `version.properties` on every commit that changes plugin code, and the plugin reports that version to CoolerControl. — ✓ commit 8615697 raised build 1→2; `cc_omen --version` = 0.1.0+2
- [x] ISC-3: The GitHub repository and its mirror are private until a publish decision, and both `main` branches equal local `main`. — ✓ gh visibility PRIVATE; both main = 8615697
- [x] ISC-4: Every issue exists on both forges under the same number and title. — ✓ #1–#10 created in order on both; open set #1, #8, #10 on both
- [x] ISC-5: Anti: no tracked file is Python, and no script or doc invokes npm or npx. — ✓ git ls-files: 0 .py; rg npm/npx only in this ISA's own criterion text
- [x] ISC-6: Anti: no tracked file, issue or commit message contains a personal name, private hostname, home-directory path or e-mail address other than a noreply address. — ✓ bun scripts/privacy-scan.ts clean (files, 2 commits, issues). It matches generic e-mail, home-path and LAN-address patterns plus the terms in the git-ignored `.privacy-terms`, as whole words ("monitor" is no hit), with `!token` lines allowing the public GitHub handle. A planted file with an e-mail, a home path and two listed terms gave 4 findings and exit 1.

### F1 · Device and channels
Why: CoolerControl sees the fans as a normal device it can drive with ordinary curves.

- [x] ISC-7: CoolerControl lists a device "HP OMEN fans" with two speed channels, "CPU fan" and "GPU fan", each 0–100 % with fixed duty enabled. — ✓ live 2026-11-18: /etc/coolercontrol/config.toml has `device-settings.e1b3fc…` with fan1 → "CPU Cooler (Balanced)", fan2 → "GPU Fan (Balanced)", assigned by the user in the UI
- [x] ISC-8: `Status` reports each fan's RPM from `fan1_input`/`fan2_input` and its duty as the driver's `pwm` reading in percent, so the floor reads 33 % like other system monitors (uncalibrated setup). — ✓ process_full_lifecycle: Status rpm 2000, duty 33 from pwm 85
- [ ] ISC-9: A curve assigned to a channel in CoolerControl reaches the plugin as `FixedDuty` calls, visible in the plugin log at debug level.

### F2 · Mode mapping
Why: 0 % means the firmware stops the fans; anything above 0 % means the curve rules.

- [x] ISC-10: When both channels are requested at 0 % (or reset), `pwm1_enable` becomes 2 and the fans reach 0 RPM on a cool machine. — ✓ live 16:40:56 and 16:41:43: en=2, fan1=0, fan2=0 at CPU 46 °C
- [x] ISC-11: When either channel is requested above 0 %, `pwm1_enable` becomes 1 and `pwm1`/`pwm2` carry the requested duties scaled to 0–255. — ✓ mode_any_nonzero_gives_manual_with_scaled_duties; process_full_lifecycle
- [x] ISC-12: The plugin never writes a pwm value while the mode is auto or max. — ✓ mode_no_pwm_write_in_auto_or_max (fake driver refuses pwm outside manual)
- [x] ISC-13: Returning from manual to auto waits for a dwell time (default 30 s) in which both channels stay at 0 %; a request above 0 % during the dwell cancels it. — ✓ dwell_holds_manual_then_returns_to_auto, dwell_is_cancelled_by_a_nonzero_request; mutation 'no dwell' caught
- [x] ISC-14: Dwell time, emergency temperature and watchdog timeout are read from the plugin's `config.json`, with defaults when it is missing or malformed. — ✓ config_* tests (5)
- [x] ISC-15: The mode logic runs against a fake sysfs directory in unit tests covering every transition (auto→manual, manual→auto with and without dwell, one channel at zero, reset, emergency). — ✓ control::tests transitions, dwell, emergency, reset, one channel zero

### F3 · Safety
Why: no failure leaves the hardware at a low fixed speed with nobody in charge.

- [x] ISC-16: On `Shutdown`, SIGTERM, SIGINT or a panic, the plugin sets `pwm1_enable` to 2 before exiting. — ✓ process_full_lifecycle (SIGTERM), process_shutdown_rpc_hands_fans_back, panic_hook_hands_fans_to_firmware; mutations caught
- [ ] ISC-17: If no RPC arrives for the watchdog timeout (default 15 s) while in manual mode, the plugin sets firmware auto and logs it.
- [ ] ISC-18: If the CPU temperature reaches the emergency limit (default 95 °C), the plugin sets `pwm1_enable` to 0 (full speed) until it falls 10 °C below the limit, whatever CoolerControl asks.
- [ ] ISC-19: After suspend and resume, the requested mode and duties are applied again within one status cycle.
- [x] ISC-20: At start the plugin refuses to run, with a clear log line, unless an hwmon device named `hp` with `pwm1_enable`, `pwm1` and `pwm2` exists. — ✓ process_refuses_without_the_fan_interface, process_refuses_without_a_cpu_sensor
- [x] ISC-21: Anti: the plugin never writes any file outside the `hp` hwmon directory, its own socket and its own plugin folder. — ✓ only hw::write_attr (3 fixed names) and hw::force_auto open files for writing; socket in /run
- [x] ISC-22: Anti: no sequence of RPCs and clock ticks leaves the fans in manual mode with no RPC received for longer than the watchdog timeout (probe: property test over random sequences against the fake sysfs). — ✓ proptest_never_manual_without_a_live_controller; mutation 'watchdog never trips' caught
- [x] ISC-34: Anti: a CPU temperature that cannot be read is never treated as cool: three failed reads in a row force full speed, and a missing sensor at start refuses to start. — ✓ temp_failure_forces_full_speed_after_three_reads, temp_failure_in_engine_forces_full_speed; mutation caught
- [x] ISC-35: The control loop runs on its own timer, so the emergency limit and the watchdog act even when no RPC arrives. — ✓ emergency_without_rpc; control loop on its own tokio interval (main.rs)
- [x] ISC-36: A panic in any thread or task sets `pwm1_enable` to 2 and ends the process, so a half-dead plugin never keeps answering RPCs. — ✓ panic_hook_hands_fans_to_firmware; hook aborts the process in service mode; mutation caught
- [x] ISC-37: On start the plugin writes firmware auto before it serves, and never trusts the state a dead instance left behind. — ✓ process_start_writes_auto_before_anything_else, process_full_lifecycle; mutation caught
- [x] ISC-38: Anti: only the hwmon device named `hp` is ever written, found by name (never by index); on a board other than 8BCA the plugin refuses to start unless `--allow-any-board` is given. — ✓ discovery_finds_hp_by_name_and_ignores_decoys, process_refuses_a_foreign_board_unless_allowed
- [x] ISC-39: Anti: the socket lives in `/run`, is owned by root with mode 0600, and a non-socket file or symlink at its path is never deleted or followed. — ✓ process_full_lifecycle (0600, owner), process_never_deletes_a_non_socket_at_the_socket_path; mutation caught
- [x] ISC-40: Anti: `config.json` cannot weaken the safety paths: values are clamped to documented bounds, unknown keys or malformed JSON fall back to defaults with a log line, and a file that is a symlink, not owned by the plugin user, or writable by group or others is ignored. — ✓ config_bad_values_are_clamped_with_a_note, config_malformed_or_unknown_keys_fall_back_to_defaults, config_unsafe_files_are_ignored; mutation caught
- [x] ISC-41: When another program changes `pwm1_enable` (for example the stock `hp` device still managed in CoolerControl), the plugin restores its state and logs a warning that names the likely cause. — ✓ foreign_writer_is_corrected_and_reported; mutation caught
- [x] ISC-42: Anti: a failed sysfs write is never recorded as applied; it is logged and retried on the next tick, and `pwm1_enable = 1` is always written before any duty. — ✓ write_failure_is_not_applied_and_retried, write_failure_retried_on_next_cycle; mutation caught
- [x] ISC-43: Anti: nothing is written to sysfs while the requested state is unchanged, and requests alternating between 0 % and 1 % every second cause at most one manual-to-auto switch per dwell window. — ✓ churn_no_writes_when_unchanged, churn_alternating_requests_switch_to_auto_at_most_once_per_dwell; mutation caught
- [x] ISC-44: Anti: neither the watchdog, the dwell nor any RPC ends emergency full speed before the CPU is at or below the limit minus 10 °C. — ✓ emergency_overrides_everything_until_limit_minus_hysteresis; mutation caught
- [x] ISC-45: After the `hp` hwmon device disappears and returns under another number (driver rebind, resume), the plugin finds it again without a restart. — ✓ rediscover_after_the_device_moves
- [x] ISC-46: Anti: no request field ever reaches a filesystem path; every `int32` duty maps into 0–255, with 0 → 0, 100 → 255 and out-of-range values clamped. — ✓ duty_scale_every_int32 (proptest), duty_scale_endpoints; unknown channel refused in process_full_lifecycle; mutation caught
- [ ] ISC-47: Anti: install labels with `semanage fcontext` plus `restorecon` (so a later `restorecon -R` keeps `bin_t`) and never touches `setenforce`; uninstall leaves no process, socket, SELinux rule or file behind.

### F4 · Install and lifecycle
Why: one command in, one command out, and it survives updates and Secure Boot.

- [ ] ISC-23: `./install.sh` builds as the user, then with sudo copies the release binary and manifest into `/var/lib/coolercontrol/plugins/cc_omen/`, sets `bin_t` with `semanage fcontext` and `restorecon`, and restarts `coolercontrold`; running it twice succeeds.
- [x] ISC-24: After install, `journalctl -u cc-plugin-cc_omen` shows the plugin started and connected, with no `203/EXEC`. — ✓ journal: "Starting cc_omen v0.1.0+3", "firmware auto set at start", 0 × 203/EXEC; binary labelled system_u:object_r:bin_t:s0
- [ ] ISC-25: `sudo ./uninstall.sh` removes the plugin folder and the SELinux rule, leaves `pwm1_enable` at 2, and restarts `coolercontrold`.
- [x] ISC-26: The README, the install script's closing message and the plugin's foreign-writer warning all say to disable the stock `hp` device in CoolerControl, and the README explains assigning curves to the plugin's channels. — ✓ README § Set up CoolerControl, install.sh line 38, engine.rs ForeignChange warning (the user reported there is no "Default Profile" entry in the UI, so the earlier wording was wrong)
- [ ] ISC-27: The plugin starts and controls the fans after a reboot with Secure Boot enabled.
- [x] ISC-28: Anti: the install script writes nothing outside `/var/lib/coolercontrol/plugins/cc_omen/` and the SELinux file-context rule. — ✓ install.sh writes plugin folder + one semanage rule only (read)

### F5 · Real-machine proof
Why: the claim is about real fans and temperatures, so it closes on the real laptop.

- [ ] ISC-29: Idle for 5 minutes with curves assigned: both fans at 0 RPM and `pwm1_enable` = 2.
- [x] ISC-30: Under CPU load the CPU fan starts within one status cycle of the CPU temperature crossing the curve's first non-zero point. — ✓ 5 Hz sampler: CPU first ≥ 50 °C at 16:42:45.780, en 2→1 at 16:42:46.232 (0.45 s)
- [ ] ISC-31: During a 10-minute all-core load the peak CPU temperature stays below the peak of the same load under firmware auto mode, both recorded in the Log.
- [x] ISC-32: After the load ends, the fans stop again once both curves are back at 0 % and the dwell has passed. — ✓ last reading ≥ 50 °C at 16:42:03.5, en 1→2 at 16:42:36.9 (dwell 30 s + CoolerControl's 2 s response delay), fans 0 RPM
- [ ] ISC-33: Killing the plugin process during load leaves `pwm1_enable` at 2, or CoolerControl restarts the plugin, within the watchdog timeout.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to | severity | tier |
|-----|------|-------|-----------|------|------------|----------|------|
| ISC-1 | bash | cargo test and release build | exit 0 | cargo test && cargo build --release | literal | critical | build |
| ISC-2 | bash | commit touching src raises build number | changed | git log -p version.properties | literal | normal | build |
| ISC-3 | bash | both remotes private, main sha equal | equal | gh repo view --json visibility | literal | normal | build |
| ISC-4 | bash | issue lists compared by number and title | identical | gh issue list | literal | normal | build |
| ISC-5 | bash | no .py files, no npm/npx | none | rg -l -e 'npm ' -e 'npx ' | literal | normal | build |
| ISC-6 | bash | personal-data scan of tree, history and issues | none | bun scripts/privacy-scan.ts | literal | critical | build |
| ISC-7 | bash | CoolerControl API lists device and channels | present | curl -sk https://localhost:11987/devices | literal | critical | integrity |
| ISC-8 | bash | Status RPC vs sysfs fan*_input | equal | cargo test --test status | literal | normal | integrity |
| ISC-9 | bash | plugin log shows FixedDuty after curve assignment | present | journalctl -u cc-plugin-cc_omen | literal | critical | integrity |
| ISC-10 | bash | unit test, then live sysfs read on a cool machine | enable=2, 0 RPM | cargo test mode | literal | critical | integrity |
| ISC-11 | bash | unit test of scaling and mode | exact | cargo test mode | literal | critical | build |
| ISC-12 | bash | unit test: no pwm write in auto or max | none | cargo test mode | literal | critical | build |
| ISC-13 | bash | unit test with fake clock | dwell honoured | cargo test dwell | literal | normal | build |
| ISC-14 | bash | config present, missing, malformed | defaults used | cargo test config | literal | normal | build |
| ISC-15 | bash | transition list covered | all pass | cargo test transitions | literal | critical | build |
| ISC-16 | bash | SIGTERM, SIGINT, panic, Shutdown write enable=2 | written | cargo test exit_paths | literal | critical | build |
| ISC-17 | bash | fake clock, then live with RPCs stopped | enable=2 | cargo test watchdog | literal | critical | integrity |
| ISC-18 | bash | fake temperature; live with a low limit in config | enable=0 then back | cargo test emergency | literal | critical | integrity |
| ISC-19 | bash | suspend, resume, read sysfs | reapplied | bun scripts/sample.ts | literal | normal | integrity |
| ISC-20 | bash | binary against empty fake sysfs | non-zero exit with message | cargo test refuse | literal | normal | build |
| ISC-21 | bash | write paths in source and under strace | only allowed paths | rg 'write\(' src | literal | critical | build |
| ISC-22 | bash | property test over random RPC and clock sequences | invariant holds | cargo test proptest | literal | critical | build |
| ISC-34 | bash | fake temp file removed or garbage for 3 ticks | enable=0 | cargo test temp_failure | literal | critical | build |
| ISC-35 | bash | no RPCs, fake temp 96 °C, ticks only | enable=0 | cargo test emergency_without_rpc | literal | critical | build |
| ISC-36 | bash | panic in a spawned thread with hook installed | enable=2 | cargo test panic_hook | literal | critical | build |
| ISC-37 | bash | start binary on fake sysfs with enable=1 | enable=2 before first RPC | cargo test --test process | literal | critical | build |
| ISC-38 | bash | fake sysfs with a decoy pwm device and a foreign board | only hp written, foreign board refused | cargo test discovery | literal | critical | build |
| ISC-39 | bash | socket mode and owner; planted regular file survives | 0600, file intact | cargo test --test process | literal | critical | build |
| ISC-40 | bash | table of bad configs and bad file permissions | clamped or ignored | cargo test config | literal | critical | build |
| ISC-41 | bash | external write of enable=1 while auto | restored and warned | cargo test foreign_writer | literal | normal | build |
| ISC-42 | bash | fake pwm write failure | not applied, retried, enable before duty | cargo test write_failure | literal | critical | build |
| ISC-43 | bash | unchanged requests and 0/1 % alternation | no writes; one switch per dwell | cargo test churn | literal | normal | build |
| ISC-44 | bash | 96 °C, watchdog expiry, RPCs, then cooling | exit only at limit−10 | cargo test emergency | literal | critical | build |
| ISC-45 | bash | hwmon dir renamed in fake sysfs | rediscovered | cargo test rediscover | literal | normal | build |
| ISC-46 | bash | property test over all int32 duties | 0..=255 | cargo test duty_scale | literal | critical | build |
| ISC-47 | bash | restorecon -R after install; uninstall leftovers | bin_t kept; nothing left | ls -Z /var/lib/coolercontrol/plugins/cc_omen | literal | normal | integrity |
| ISC-23 | bash | install run twice, label checked | exit 0, bin_t | ls -Z /var/lib/coolercontrol/plugins/cc_omen | literal | critical | integrity |
| ISC-24 | bash | service log after install | started, no 203 | journalctl -u cc-plugin-cc_omen | literal | critical | integrity |
| ISC-25 | bash | uninstall, then folder, rule and sysfs | removed, enable=2 | cat /sys/class/hwmon/*/pwm1_enable | literal | normal | integrity |
| ISC-26 | bash | disable instruction present in all three places | present | rg -n -i 'disable the stock' README.md install.sh src/engine.rs | literal | normal | build |
| ISC-27 | bash | Secure Boot on, then ISC-24 and ISC-10 | all true | mokutil --sb-state | literal | critical | integrity |
| ISC-28 | bash | paths written by install script | only listed | rg -n -e install -e cp -e mkdir install.sh | literal | normal | build |
| ISC-29 | bash | 5-minute idle sampling | 0 RPM, enable=2 | bun scripts/sample.ts --idle 300 | literal | critical | integrity |
| ISC-30 | bash | sampling under load | start within one cycle | bun scripts/sample.ts --load | literal | critical | integrity |
| ISC-31 | bash | 10-minute load, plugin vs firmware auto | lower peak | bun scripts/sample.ts --load 600 | literal | critical | integrity |
| ISC-32 | bash | sampling after load | 0 RPM after dwell | bun scripts/sample.ts --cooldown | literal | normal | integrity |
| ISC-33 | bash | kill -9 during load, sample sysfs | enable=2 or restarted | bun scripts/sample.ts --kill | literal | critical | integrity |

## Not yet specified

- Whether CoolerControl restarts a killed device plugin by itself, and how fast (decides how ISC-33 closes).
- Whether CoolerControl calls `Status` often enough to serve as the watchdog heartbeat, or the watchdog must count only `FixedDuty` calls.
- What the firmware does with `pwm1_enable` across suspend (ISC-19 assumes it may reset).
- When and how the repository goes public (needs an explicit go-ahead).

## Decisions

- 2026-11-18 · approach: CoolerControl device plugin over a patched kernel module. No rebuild on every kernel update, no MOK signing, Secure Boot can stay on. Cost: a channel at 0 % while the other runs sits at the driver floor.
- 2026-11-18 · language: Rust from the official device-service template, as a root service binary; TypeScript/bun for the sampler and helper scripts. Licence GPL-3.0, following the template.
- 2026-11-18 · plugin id `cc_omen`, repository name `CC-Omen-plugin`.
- 2026-11-18 · no milestones: about ten issues, tracked from the plan issue #1.
- 2026-11-18 · public-ready from day one: no personal data anywhere (ISC-6).
- 2026-11-18 · finding: the in-tree driver clamps manual writes to `min_rpm` from the BIOS fan table (`clamp_val(rpm, priv->min_rpm, priv->max_rpm)` in `hp_wmi_hwmon_write`); reproduced when a manual write of 0 read back as `pwm1=85` at 2000 RPM. Firmware auto at 49 °C gave 0 RPM on both fans.
- 2026-11-18 · absence review: a fresh Opus reviewer saw only the ladder, problem, constraints and out-of-scope; 15 findings, adopted as ISC-34–47 (two folded into ISC-36 and ISC-47). Rebutted: NaN duties (the protocol duty is `int32`). SIGKILL cannot be caught by any handler; the README says so and ISC-33 tests what happens after it.
- 2026-11-18 · CoolerControl stays uncalibrated: a channel shows pwm/255 as its percent, so the driver floor (pwm 85, about 2000 RPM) reads as 33 %. Curve values from 1 % to 33 % all run at the floor.
- 2026-11-18 · mutation run: the first pass let "SIGTERM does not hand back" survive because `stop()` already writes auto through the controller (an equivalent mutant); the mutation now removes the `stop()` call itself and is caught. A refuse-to-start test hung for 15 minutes on a mutant that kept running; refuse tests now kill the plugin after 10 s and fail.
- 2026-11-18 · Status reports duty from the driver's pwm reading, not the requested duty, so CoolerControl shows what the fans really do (33 % at the floor) even in firmware auto.
- 2026-11-18 · an unreadable temperature means full speed, not firmware auto: a controller that cannot see the temperature should not bet on the firmware's late reaction.

## Status

Built and proven against a fake sysfs: 31/47 criteria closed. 13 of 13 deliberate breaks of a safety guard are caught (`bun scripts/mutate.ts`). Open: everything that needs the plugin installed in CoolerControl and the real fans (ISC-7, 9, 10, 17–19, 23–25, 27, 29–33, 47). Next: the user runs `./install.sh`, then the live checks.
