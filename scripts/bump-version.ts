// Pre-commit: every commit that changes plugin code raises the build number in
// version.properties and mirrors the version into manifest.toml, so the version CoolerControl
// shows always identifies the code that runs.
import { $ } from "bun";

const staged = (await $`git diff --cached --name-only`.text()).split("\n").filter(Boolean);
const code = staged.some((f) => /^(src\/|proto\/|build\.rs$|Cargo\.(toml|lock)$|manifest\.toml$)/.test(f));
if (!code) process.exit(0);

const props = await Bun.file("version.properties").text();
const version = props.match(/^version=(.+)$/m)?.[1]?.trim();
const build = Number(props.match(/^build=(\d+)$/m)?.[1]);
if (!version || !Number.isInteger(build)) {
  console.error("version.properties: needs version=… and build=N");
  process.exit(1);
}
const next = build + 1;
await Bun.write("version.properties", `version=${version}\nbuild=${next}\n`);
const manifest = await Bun.file("manifest.toml").text();
await Bun.write("manifest.toml", manifest.replace(/^version = ".*"$/m, `version = "${version}+${next}"`));
await $`git add version.properties manifest.toml`;
console.log(`cc_omen ${version}+${next}`);
