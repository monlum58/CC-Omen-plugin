// The repository is meant to become public. This scan fails if a tracked file, a commit
// (message, author or committer) or a GitHub issue carries personal data:
//   - e-mail addresses other than noreply ones
//   - home-directory paths and private LAN addresses
//   - any term listed in the untracked file .privacy-terms (one per line: names, hostnames, …),
//     matched as a whole word; a line "!token" allows that exact token (e.g. the public GitHub handle)
// Usage: bun scripts/privacy-scan.ts [--no-issues]
import { $ } from "bun";

const lines = (await Bun.file(".privacy-terms").exists())
  ? (await Bun.file(".privacy-terms").text()).split("\n").map((s) => s.trim()).filter((s) => s && !s.startsWith("#"))
  : [];
const allowed = lines.filter((l) => l.startsWith("!")).map((l) => l.slice(1));
const terms = lines.filter((l) => !l.startsWith("!"));
const esc = (t: string) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const generic: [string, RegExp][] = [
  ["e-mail", /(?<![A-Za-z0-9._%+-])(?!(?:\d+\+)?[A-Za-z0-9._%-]*noreply@)[A-Za-z0-9._%+-]+@(?![A-Za-z0-9.-]*example\.)[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g],
  ["home path", /\/(var\/)?home\/[A-Za-z0-9_.-]+/g],
  ["private address", /\b(192\.168|10\.\d{1,3}|172\.(1[6-9]|2\d|3[01]))\.\d{1,3}\.\d{1,3}\b/g],
];

const hits: string[] = [];
function scan(where: string, text: string) {
  for (const [kind, re] of generic) {
    for (const m of text.matchAll(re)) {
      if (kind === "e-mail" && /noreply/i.test(m[0])) continue;
      hits.push(`${where}: ${kind} "${m[0]}"`);
    }
  }
  let rest = text;
  for (const a of allowed) rest = rest.replaceAll(new RegExp(esc(a), "gi"), " ");
  for (const [i, t] of terms.entries()) {
    if (new RegExp(`\\b${esc(t)}\\b`, "i").test(rest)) hits.push(`${where}: listed term #${i + 1}`);
  }
}

const self = "scripts/privacy-scan.ts";
for (const f of (await $`git ls-files`.text()).split("\n").filter(Boolean)) {
  if (f === self || f === "LICENSE") continue;
  scan(f, await Bun.file(f).text());
}
const log = await $`git log --all --format=%H%n%an%n%ae%n%cn%n%ce%n%B%n--END--`.text();
for (const entry of log.split("--END--")) {
  const sha = entry.trim().split("\n")[0]?.slice(0, 8);
  if (sha) scan(`commit ${sha}`, entry);
}
if (!process.argv.includes("--no-issues")) {
  const r = await $`gh issue list --state all --limit 500 --json number,title,body`.quiet().nothrow();
  if (r.exitCode === 0) {
    for (const i of JSON.parse(r.stdout.toString())) scan(`issue #${i.number}`, `${i.title}\n${i.body}`);
  } else {
    console.log("issues not scanned (gh unavailable)");
  }
}

if (hits.length) {
  console.log(hits.join("\n"));
  console.log(`${hits.length} finding(s)`);
  process.exit(1);
}
console.log(`clean: files, ${log.split("--END--").length - 1} commits${process.argv.includes("--no-issues") ? "" : ", issues"}; ${terms.length} listed terms`);
