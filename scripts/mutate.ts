// Mutation check: break each safety guard on purpose and require the test suite to fail.
// A mutation the tests do not catch means a guard that nothing proves.
// Usage: bun scripts/mutate.ts            (all)
//        bun scripts/mutate.ts --restore  (put files back after an interrupted run)
import { $ } from "bun";

type Mutation = { name: string; file: string; from: string; to: string };

const mutations: Mutation[] = [
  {
    name: "watchdog never trips",
    file: "src/control.rs",
    from: "if silent && (self.requested != [0, 0] || self.applied == Some(Mode::Manual)) {",
    to: "if false && silent {",
  },
  {
    name: "emergency ends without hysteresis",
    file: "src/control.rs",
    from: "t <= self.cfg.emergency_c - self.cfg.hysteresis_c",
    to: "t < self.cfg.emergency_c",
  },
  {
    name: "unreadable temperature counts as cool",
    file: "src/control.rs",
    from: "self.emergency || self.temp_failures >= TEMP_FAILURES_LIMIT",
    to: "self.emergency",
  },
  {
    name: "no dwell before auto",
    file: "src/control.rs",
    from: "if now.saturating_sub(since) >= self.cfg.dwell_ms { Mode::Auto } else { Mode::Manual }",
    to: "{ let _ = since; Mode::Auto }",
  },
  {
    name: "write failure recorded as applied",
    file: "src/engine.rs",
    from: "                self.ctrl.failed();\n            }\n        }\n    }",
    to: "                self.ctrl.commit(&plan);\n            }\n        }\n    }",
  },
  {
    name: "panic hook does not hand back",
    file: "src/engine.rs",
    from: "let _ = hw::force_auto(&p);",
    to: "let _ = &p;",
  },
  {
    name: "no firmware auto at start",
    file: "src/engine.rs",
    from: "fans.set_mode(Mode::Auto)?;\n        self.ctrl.assume_applied(Mode::Auto);",
    to: "self.ctrl.assume_applied(Mode::Auto);",
  },
  {
    name: "SIGTERM does not hand back",
    file: "src/main.rs",
    from: "engine.lock().unwrap_or_else(|e| e.into_inner()).stop();",
    to: "let _ = &engine;",
  },
  {
    name: "unchanged state rewritten every tick",
    file: "src/control.rs",
    from: "let mode_ok = self.applied == Some(target) && actual == Some(target);",
    to: "let mode_ok = false;",
  },
  {
    name: "foreign mode change ignored",
    file: "src/control.rs",
    from: "let mode_ok = self.applied == Some(target) && actual == Some(target);",
    to: "let mode_ok = self.applied == Some(target);",
  },
  {
    name: "unsafe config file accepted",
    file: "src/config.rs",
    from: "if meta.mode() & 0o022 != 0 {",
    to: "if false {",
  },
  {
    name: "duty not clamped",
    file: "src/control.rs",
    from: "duty.clamp(0, 100) as u8",
    to: "duty as u8",
  },
  {
    name: "non-socket at socket path deleted",
    file: "src/main.rs",
    from: "bail!(\"{} exists and is not a socket; refusing to touch it\", path.display());",
    to: "std::fs::remove_file(path)?;",
  },
];

const backup = ".harness/mutating.json";

async function restore() {
  const f = Bun.file(backup);
  if (!(await f.exists())) return;
  const saved: Record<string, string> = await f.json();
  for (const [file, text] of Object.entries(saved)) await Bun.write(file, text);
  await $`rm ${backup}`.quiet();
  console.log("restored", Object.keys(saved).join(", "));
}

if (process.argv.includes("--restore")) {
  await restore();
  process.exit(0);
}
await restore();
await $`mkdir -p .harness`.quiet();

let survived = 0;
for (const m of mutations) {
  const original = await Bun.file(m.file).text();
  if (!original.includes(m.from)) {
    console.log(`✗ ${m.name}: anchor not found in ${m.file}`);
    survived++;
    continue;
  }
  await Bun.write(backup, JSON.stringify({ [m.file]: original }));
  await Bun.write(m.file, original.replace(m.from, m.to));
  const r = await $`cargo test --quiet`.quiet().nothrow();
  await Bun.write(m.file, original);
  await $`rm ${backup}`.quiet();
  const out = r.stdout.toString() + r.stderr.toString();
  const failing = [...out.matchAll(/^test (\S+) \.\.\. FAILED/gm)].map((x) => x[1]);
  const fromProcess = out.match(/^\s{4}(process_\w+)$/gm)?.map((s) => s.trim()) ?? [];
  const caught = r.exitCode !== 0;
  if (!caught) survived++;
  const which = [...new Set([...failing, ...fromProcess])].join(", ") || (caught ? "build or test failure" : "");
  console.log(`${caught ? "✓ caught" : "✗ SURVIVED"} ${m.name}${which ? ` — ${which}` : ""}`);
}
console.log(survived === 0 ? `all ${mutations.length} mutations caught` : `${survived} of ${mutations.length} survived`);
process.exit(survived === 0 ? 0 : 1);
