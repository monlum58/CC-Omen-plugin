#!/usr/bin/env bash
# Live checks on the real laptop for the criteria a fake sysfs cannot prove.
#   sudo scripts/live-checks.sh                 all phases, about 22 minutes; leave the laptop alone
#   sudo ONLY="17 31" scripts/live-checks.sh    only the named phases (criterion numbers)
#
# Safety: a guard watches the CPU every 0.5 s during the whole run. At GUARD_C it kills
# the load and sets the fans to full speed. Light load lasts at most 60 s, full load at most
# LOAD_S (emergency phase) or COMPARE_S at COMPARE_THREADS (comparison), and the firmware-auto run happens
# once, last, after a cool-down. On any exit the script stops its load, resumes or restarts
# coolercontrold, and removes its test settings.
set -u

GUARD_C=85
LOAD_S=90
COMPARE_S=90
COMPARE_THREADS=4
IDLE_S=300
ID=cc_omen
HERE="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN_DIR="$(readlink -f /var/lib/coolercontrol/plugins)/$ID"
LOG_DIR="$HERE/.harness"
LOG="$LOG_DIR/live-$(date +%Y%m%d-%H%M%S).log"
ONLY="${ONLY:-}"

[ "$(id -u)" -eq 0 ] || exec sudo ONLY="$ONLY" "$0" "$@"
mkdir -p "$LOG_DIR"

HP=""
for d in /sys/class/hwmon/hwmon*; do [ "$(cat "$d/name" 2>/dev/null)" = hp ] && HP="$d"; done
CPU=""
for d in /sys/class/hwmon/hwmon*; do [ "$(cat "$d/name" 2>/dev/null)" = k10temp ] && CPU="$d/temp1_input"; done
[ -n "$HP" ] && [ -n "$CPU" ] || { echo "hp fan device or k10temp not found" >&2; exit 1; }

want() { [ -z "$ONLY" ] || [[ " $ONLY " == *" $1 "* ]]; }
log() { echo "$(date +%T) $*" | tee -a "$LOG"; }
result() { echo "RESULT $1 $2 — $3" | tee -a "$LOG"; }
cpu() { echo $(( $(cat "$CPU") / 1000 )); }
cpu_tenths() { echo $(( $(cat "$CPU") / 100 )); }
en() { cat "$HP/pwm1_enable"; }
rpm() { echo "$(cat "$HP/fan1_input")/$(cat "$HP/fan2_input")"; }
state() { echo "cpu=$(cpu_tenths) en=$(en) rpm=$(rpm)"; }
cc_pid() { systemctl show -p MainPID --value coolercontrold; }
plugin_pid() { systemctl show -p MainPID --value "cc-plugin-$ID"; }

LOAD_PIDS=()
load_start() { # $1 = number of busy processes
  for _ in $(seq 1 "$1"); do yes >/dev/null & LOAD_PIDS+=($!); done
}
load_stop() {
  # Only our own load processes: a bare `wait` would also wait for the guard loop forever.
  if [ ${#LOAD_PIDS[@]} -gt 0 ]; then
    kill "${LOAD_PIDS[@]}" 2>/dev/null
    wait "${LOAD_PIDS[@]}" 2>/dev/null
  fi
  LOAD_PIDS=()
}

GUARD_FLAG="$LOG_DIR/guard-tripped"
rm -f "$GUARD_FLAG"
guard() {
  while :; do
    if [ "$(cpu)" -ge "$GUARD_C" ]; then
      pkill -x yes 2>/dev/null
      echo 0 > "$HP/pwm1_enable"
      [ -f "$GUARD_FLAG" ] || { touch "$GUARD_FLAG"; echo "$(date +%T) GUARD: CPU at $(cpu) °C, load killed, fans full speed" | tee -a "$LOG"; }
    fi
    sleep 0.5
  done
}
guard & GUARD_PID=$!

TEST_CONFIG=0
RESTART_AT_END=0
cleanup() {
  load_stop
  pkill -x yes 2>/dev/null
  kill "$GUARD_PID" 2>/dev/null
  local cc; cc=$(cc_pid)
  [ "$cc" != 0 ] && kill -CONT "$cc" 2>/dev/null
  if [ "$TEST_CONFIG" = 1 ]; then unlink "$PLUGIN_DIR/config.json" 2>/dev/null; fi
  if [ -f "$PLUGIN_DIR/manifest.toml" ] && grep -q -- '"--debug"' "$PLUGIN_DIR/manifest.toml"; then
    sed -i 's/, "--debug"//' "$PLUGIN_DIR/manifest.toml"
    RESTART_AT_END=1
  fi
  if ! systemctl is-active -q coolercontrold; then
    systemctl start coolercontrold
  elif [ "$TEST_CONFIG" = 1 ] || [ "$RESTART_AT_END" = 1 ]; then
    systemctl restart coolercontrold
  fi
  # Nothing controls the fans without the plugin: make sure the firmware does.
  systemctl is-active -q "cc-plugin-$ID" || echo 2 > "$HP/pwm1_enable"
  log "cleanup done: $(state); full log: $LOG"
  chown -R --reference="$HERE" "$LOG_DIR" 2>/dev/null
}
trap cleanup EXIT
trap 'exit 130' INT TERM

wait_until() { # $1 timeout s, $2… condition; returns 0 when true
  local t=$1; shift
  local end=$((SECONDS + t))
  while [ $SECONDS -lt "$end" ]; do "$@" && return 0; sleep 0.2; done
  return 1
}
is_en() { [ "$(en)" = "$1" ]; }
cool_below() { [ "$(cpu)" -lt "$1" ]; }
plugin_started_since() { journalctl -u "cc-plugin-$ID" --since "@$1" --no-pager -q | grep -q "firmware auto set at start"; }

cool_down() { # $1 target °C, $2 timeout s
  log "cooling down below $1 °C ($(state))"
  wait_until "$2" cool_below "$1" || log "still $(cpu) °C after $2 s, continuing"
}

to_manual() { # light load until the curve puts the fans in manual
  load_start 4
  if wait_until 60 is_en 1; then load_stop; return 0; fi
  load_stop
  return 1
}

load_peak() { # $1 seconds, $2 busy threads; prints "peak end" in tenths °C
  local peak=0 t end=$((SECONDS + $1))
  load_start "$(nproc)"
  while [ $SECONDS -lt $end ] && [ ! -f "$GUARD_FLAG" ]; do
    t=$(cpu_tenths); [ "$t" -gt "$peak" ] && peak=$t
    sleep 0.5
  done
  t=$(cpu_tenths)
  load_stop
  echo "$peak $t"
}

log "live checks start: $(state); Secure Boot: $(mokutil --sb-state 2>/dev/null); phases: ${ONLY:-all}"
systemctl is-active -q coolercontrold || { log "coolercontrold not running"; exit 1; }
systemctl is-active -q "cc-plugin-$ID" || { log "plugin not running"; exit 1; }

if want 29; then # 5 minutes idle at 0 RPM in firmware auto.
  cool_down 50 180
  wait_until 90 is_en 2
  bad=0; n=0; end=$((SECONDS + IDLE_S))
  while [ $SECONDS -lt $end ]; do
    n=$((n + 1))
    if [ "$(en)" != 2 ] || [ "$(rpm)" != "0/0" ]; then bad=$((bad + 1)); log "idle sample off: $(state)"; fi
    sleep 2
  done
  result ISC-29 "$([ $bad -eq 0 ] && echo PASS || echo FAIL)" "$((n - bad))/$n idle samples at en=2 and 0/0 RPM over ${IDLE_S}s"
fi

if want 9; then # FixedDuty requests visible at debug level.
  t0=$(date +%s)
  sed -i 's/args = \["--socket", "\/run\/coolercontrol-plugin-cc_omen.sock"\]/args = ["--socket", "\/run\/coolercontrol-plugin-cc_omen.sock", "--debug"]/' "$PLUGIN_DIR/manifest.toml"
  systemctl restart coolercontrold
  wait_until 30 plugin_started_since "$t0"
  to_manual
  n=$(journalctl -u "cc-plugin-$ID" --since "@$t0" --no-pager -q | grep -c "FixedDuty")
  result ISC-9 "$([ "$n" -gt 0 ] && echo PASS || echo FAIL)" "$n FixedDuty lines at debug level since $(date -d "@$t0" +%T)"
  sed -i 's/, "--debug"//' "$PLUGIN_DIR/manifest.toml"
  RESTART_AT_END=1
  cool_down 52 120
fi

if want 17; then # watchdog: CoolerControl frozen while the fans are manual.
  if to_manual; then
    cc=$(cc_pid); kill -STOP "$cc"; t0=$SECONDS; ts=$(date +%s)
    log "coolercontrold frozen with fans manual ($(state))"
    if wait_until 30 is_en 2; then dt=$((SECONDS - t0)); else dt=-1; fi
    kill -CONT "$cc"
    warned=$(journalctl -u "cc-plugin-$ID" --since "@$ts" --no-pager -q | grep -c "no request from CoolerControl")
    ok=$([ "$dt" -ge 14 ] && [ "$dt" -le 20 ] && [ "$warned" -gt 0 ] && echo PASS || echo FAIL)
    result ISC-17 "$ok" "firmware auto ${dt}s after CoolerControl went silent (watchdog 15 s), warning logged: $warned"
  else
    result ISC-17 FAIL "could not reach manual mode with light load within 60 s ($(state))"
  fi
  cool_down 52 120
fi

if want 33; then # kill -9 while manual.
  if to_manual; then
    old=$(plugin_pid); ts=$(date +%s); t0=$SECONDS
    kill -9 "$old"
    log "plugin $old killed with SIGKILL ($(state))"
    if wait_until 60 plugin_started_since "$ts"; then
      dt=$((SECONDS - t0)); result ISC-33 PASS "CoolerControl restarted the plugin after ${dt}s; it wrote firmware auto at start (new pid $(plugin_pid))"
    else
      echo 2 > "$HP/pwm1_enable"; result ISC-33 FAIL "no restart within 60 s; fans stayed manual until the script set auto"
    fi
  else
    result ISC-33 FAIL "could not reach manual mode with light load"
  fi
  cool_down 52 120
fi

if want 18; then # emergency at a test limit of 70 °C (the lowest allowed), released at or below 60 °C.
  printf '{"emergency_celsius":70}\n' > "$PLUGIN_DIR/config.json"; chmod 644 "$PLUGIN_DIR/config.json"; TEST_CONFIG=1
  t0=$(date +%s); systemctl restart coolercontrold; wait_until 30 plugin_started_since "$t0"
  load_start "$(nproc)"
  if wait_until "$LOAD_S" is_en 0; then
    at=$(cpu_tenths); load_stop
    log "emergency full speed at $at tenths °C"
    exit_t=""
    end=$((SECONDS + 240))
    while [ $SECONDS -lt $end ]; do
      if ! is_en 0; then exit_t=$(cpu_tenths); break; fi
      sleep 0.2
    done
    ok=$([ -n "$exit_t" ] && [ "$exit_t" -le 605 ] && echo PASS || echo FAIL)
    result ISC-18 "$ok" "full speed at $at, released at ${exit_t:-never} (tenths °C; limit 70, release at or below 60)"
  else
    load_stop; result ISC-18 FAIL "no full speed within ${LOAD_S}s (CPU $(cpu) °C)"
  fi
  unlink "$PLUGIN_DIR/config.json"; TEST_CONFIG=0
  t0=$(date +%s); systemctl restart coolercontrold; wait_until 30 plugin_started_since "$t0"
fi

if want 31; then # COMPARE_S of medium load (COMPARE_THREADS busy) with the plugin, then the same with firmware auto.
  cool_down 50 300
  rm -f "$GUARD_FLAG"
  start_p=$(cpu_tenths); read -r peak_p end_p < <(load_peak "$COMPARE_S" "$COMPARE_THREADS")
  guard_p=0; [ -f "$GUARD_FLAG" ] && guard_p=1
  log "plugin run: start $start_p, peak $peak_p, end $end_p (tenths °C), guard tripped: $guard_p"
  cool_down 50 300
  rm -f "$GUARD_FLAG"
  systemctl stop coolercontrold
  wait_until 10 is_en 2
  start_f=$(cpu_tenths)
  log "firmware auto run, coolercontrold stopped ($(state))"
  read -r peak_f end_f < <(load_peak "$COMPARE_S" "$COMPARE_THREADS")
  guard_f=0; [ -f "$GUARD_FLAG" ] && guard_f=1
  systemctl start coolercontrold
  log "firmware run: start $start_f, peak $peak_f, end $end_f (tenths °C), guard tripped: $guard_f"
  if [ "$guard_p" = 1 ]; then ok=FAIL
  elif [ "$guard_f" = 1 ] || [ "$peak_p" -lt "$peak_f" ]; then ok=PASS
  else ok=FAIL; fi
  result ISC-31 "$ok" "${COMPARE_S}s with ${COMPARE_THREADS} busy threads: plugin start/peak $start_p/$peak_p vs firmware auto $start_f/$peak_f tenths °C; guard tripped plugin=$guard_p firmware=$guard_f"
  rm -f "$GUARD_FLAG"
  cool_down 55 300
fi

if want 47; then # a full relabel keeps bin_t.
  restorecon -R /var/lib/coolercontrol
  label=$(/usr/bin/ls -Z "$PLUGIN_DIR/$ID" | awk '{print $1}')
  result ISC-47a "$(echo "$label" | grep -q ':bin_t:' && echo PASS || echo FAIL)" "after restorecon -R: $label"
fi

if want 25; then # uninstall leaves nothing behind.
  "$HERE/uninstall.sh" >>"$LOG" 2>&1; rc=$?
  left=""
  [ -e "$PLUGIN_DIR" ] && left="$left folder"
  semanage fcontext -l -C 2>/dev/null | grep -q "$ID" && left="$left selinux-rule"
  pgrep -x "$ID" >/dev/null && left="$left process"
  [ -e "/run/coolercontrol-plugin-$ID.sock" ] && left="$left socket"
  result ISC-25 "$([ $rc -eq 0 ] && [ -z "$left" ] && is_en 2 && echo PASS || echo FAIL)" "uninstall exit $rc, en=$(en), left behind:${left:- nothing}"
  result ISC-47b "$([ -z "$left" ] && echo PASS || echo FAIL)" "no process, socket, rule or file left:${left:- nothing}"
fi

if want 23 || want 25; then # install twice (always after an uninstall, so the plugin comes back).
  "$HERE/install.sh" >>"$LOG" 2>&1; rc1=$?
  "$HERE/install.sh" >>"$LOG" 2>&1; rc2=$?
  label=$(/usr/bin/ls -Z "$PLUGIN_DIR/$ID" 2>/dev/null | awk '{print $1}')
  result ISC-23 "$([ $rc1 -eq 0 ] && [ $rc2 -eq 0 ] && echo "$label" | grep -q ':bin_t:' && echo PASS || echo FAIL)" "install exits $rc1/$rc2, label $label, plugin $(systemctl is-active "cc-plugin-$ID")"
fi

log "all checks done: $(state)"
grep '^RESULT' "$LOG"
