---
name: "CC-Omen-plugin"
task: "CoolerControl device plugin that lets fan curves stop the HP OMEN laptop fans"
slug: 20261118-154500_cc-omen-plugin
project: CC-Omen-plugin
phase: marking
progress: 0/32
started: 2026-11-18T15:45:00+01:00
updated: 2026-11-18T15:45:00+01:00
principal_stated_goal: 'CoolerControl is great, but it cannot stop the fans completely. Convince CoolerControl to stop the fans if the system temperature is low, while the fans still react early enough to keep the hardware cool.'
principal_stated_goal_source: conversation 2026-11-18
principal_stated_goal_signal: 3
principal_stated_goal_locked: 2026-11-18T15:45:00+01:00
context_sufficient: true
interview_invoked: false
absence_review: skipped — plan-only commit; owed before F2/F3 code is written
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

- [ ] ISC-1: `cargo test` and `cargo build --release` pass on a clean checkout.
- [ ] ISC-2: A git pre-commit hook raises the build number in `version.properties` on every commit that changes plugin code, and the plugin reports that version to CoolerControl.
- [ ] ISC-3: The GitHub repository and its mirror are private until a publish decision, and both `main` branches equal local `main`.
- [ ] ISC-4: Every issue exists on both forges under the same number and title.
- [ ] ISC-5: Anti: no tracked file is Python, and no script or doc invokes npm or npx.
- [ ] ISC-6: Anti: no tracked file, issue or commit message contains a personal name, private hostname, home-directory path or e-mail address other than the GitHub noreply address.

### F1 · Device and channels
Why: CoolerControl sees the fans as a normal device it can drive with ordinary curves.

- [ ] ISC-7: CoolerControl lists a device "HP OMEN fans" with two speed channels, "CPU fan" and "GPU fan", each 0–100 % with fixed duty enabled.
- [ ] ISC-8: `Status` reports each fan's RPM from `fan1_input`/`fan2_input` and the duty last requested for that channel.
- [ ] ISC-9: A curve assigned to a channel in CoolerControl reaches the plugin as `FixedDuty` calls, visible in the plugin log at debug level.

### F2 · Mode mapping
Why: 0 % means the firmware stops the fans; anything above 0 % means the curve rules.

- [ ] ISC-10: When both channels are requested at 0 % (or reset), `pwm1_enable` becomes 2 and the fans reach 0 RPM on a cool machine.
- [ ] ISC-11: When either channel is requested above 0 %, `pwm1_enable` becomes 1 and `pwm1`/`pwm2` carry the requested duties scaled to 0–255.
- [ ] ISC-12: The plugin never writes a pwm value while the mode is auto or max.
- [ ] ISC-13: Returning from manual to auto waits for a dwell time (default 30 s) in which both channels stay at 0 %; a request above 0 % during the dwell cancels it.
- [ ] ISC-14: Dwell time, emergency temperature and watchdog timeout are read from the plugin's `config.json`, with defaults when it is missing or malformed.
- [ ] ISC-15: The mode logic runs against a fake sysfs directory in unit tests covering every transition (auto→manual, manual→auto with and without dwell, one channel at zero, reset, emergency).

### F3 · Safety
Why: no failure leaves the hardware at a low fixed speed with nobody in charge.

- [ ] ISC-16: On `Shutdown`, SIGTERM, SIGINT or a panic, the plugin sets `pwm1_enable` to 2 before exiting.
- [ ] ISC-17: If no RPC arrives for the watchdog timeout (default 15 s) while in manual mode, the plugin sets firmware auto and logs it.
- [ ] ISC-18: If the CPU temperature reaches the emergency limit (default 95 °C), the plugin sets `pwm1_enable` to 0 (full speed) until it falls 10 °C below the limit, whatever CoolerControl asks.
- [ ] ISC-19: After suspend and resume, the requested mode and duties are applied again within one status cycle.
- [ ] ISC-20: At start the plugin refuses to run, with a clear log line, unless an hwmon device named `hp` with `pwm1_enable`, `pwm1` and `pwm2` exists.
- [ ] ISC-21: Anti: the plugin never writes any file outside the `hp` hwmon directory and its own plugin folder.
- [ ] ISC-22: Anti: no sequence of RPCs and clock ticks leaves the fans in manual mode with no RPC received for longer than the watchdog timeout (probe: property test over random sequences against the fake sysfs).

### F4 · Install and lifecycle
Why: one command in, one command out, and it survives updates and Secure Boot.

- [ ] ISC-23: `sudo ./install.sh` builds or copies the release binary and manifest into `/var/lib/coolercontrol/plugins/cc_omen/`, sets `bin_t` with `semanage fcontext` and `restorecon`, and restarts `coolercontrold`; running it twice succeeds.
- [ ] ISC-24: After install, `journalctl -u cc-plugin-cc_omen` shows the plugin started and connected, with no `203/EXEC`.
- [ ] ISC-25: `sudo ./uninstall.sh` removes the plugin folder and the SELinux rule, leaves `pwm1_enable` at 2, and restarts `coolercontrold`.
- [ ] ISC-26: The README explains keeping the stock `hp` device unmanaged in CoolerControl and assigning curves to the plugin's channels.
- [ ] ISC-27: The plugin starts and controls the fans after a reboot with Secure Boot enabled.
- [ ] ISC-28: Anti: the install script writes nothing outside `/var/lib/coolercontrol/plugins/cc_omen/` and the SELinux file-context rule.

### F5 · Real-machine proof
Why: the claim is about real fans and temperatures, so it closes on the real laptop.

- [ ] ISC-29: Idle for 5 minutes with curves assigned: both fans at 0 RPM and `pwm1_enable` = 2.
- [ ] ISC-30: Under CPU load the CPU fan starts within one status cycle of the CPU temperature crossing the curve's first non-zero point.
- [ ] ISC-31: During a 10-minute all-core load the peak CPU temperature stays below the peak of the same load under firmware auto mode, both recorded in the Log.
- [ ] ISC-32: After the load ends, the fans stop again once both curves are back at 0 % and the dwell has passed.
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
| ISC-23 | bash | install run twice, label checked | exit 0, bin_t | ls -Z /var/lib/coolercontrol/plugins/cc_omen | literal | critical | integrity |
| ISC-24 | bash | service log after install | started, no 203 | journalctl -u cc-plugin-cc_omen | literal | critical | integrity |
| ISC-25 | bash | uninstall, then folder, rule and sysfs | removed, enable=2 | cat /sys/class/hwmon/*/pwm1_enable | literal | normal | integrity |
| ISC-26 | bash | README section present | present | rg -n 'Default Profile' README.md | literal | normal | build |
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
- 2026-11-18 · absence review skipped for the plan commit; owed before F2/F3 code.

## Status

Plan only. Nothing is built or installed.
