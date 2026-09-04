import assert from "node:assert";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { describe, it } from "node:test";

const ROOT = path.dirname(new URL(import.meta.url).pathname);
const SCRIPT = path.join(ROOT, "install.sh");
const target = process.arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
const archiveName = `clockwork-${target}.tar.gz`;

function setup({ badChecksum = false } = {}) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "clockwork-release-install-"));
  const home = path.join(tmp, "home");
  const fakeBin = path.join(tmp, "bin");
  const release = path.join(tmp, "release");
  const payload = path.join(tmp, `clockwork-${target}`);
  fs.mkdirSync(home);
  fs.mkdirSync(fakeBin);
  fs.mkdirSync(release);
  fs.mkdirSync(payload);

  fs.writeFileSync(path.join(payload, "clockwork"), "#!/bin/sh\n[ \"${1:-}\" = \"--version\" ] && echo 'clockwork 0.1.0'\n", { mode: 0o755 });
  fs.cpSync(path.join(ROOT, "install.mjs"), path.join(payload, "install.mjs"));
  fs.cpSync(path.join(ROOT, "services"), path.join(payload, "services"), { recursive: true });

  const archive = path.join(release, archiveName);
  const packed = spawnSync("tar", ["-czf", archive, "-C", tmp, path.basename(payload)], { encoding: "utf8" });
  assert.strictEqual(packed.status, 0, packed.stderr);
  const checksum = badChecksum
    ? "0".repeat(64)
    : crypto.createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
  fs.writeFileSync(path.join(release, "sha256.sum"), `${checksum} *${archiveName}\n`);

  const curl = path.join(fakeBin, "curl");
  fs.writeFileSync(curl, `#!/bin/sh
set -eu
output=''
previous=''
url=''
for argument in "$@"; do
  if [ "$previous" = '--output' ]; then output=$argument; previous=''; continue; fi
  [ "$argument" = '--output' ] && { previous='--output'; continue; }
  url=$argument
done
case "$url" in
  *sha256.sum) source="$CLOCKWORK_TEST_RELEASE/sha256.sum" ;;
  *${archiveName}) source="$CLOCKWORK_TEST_RELEASE/${archiveName}" ;;
  *) echo "unexpected curl URL: $url" >&2; exit 2 ;;
esac
cp "$source" "$output"
`, { mode: 0o755 });

  const run = (args) => spawnSync("sh", [SCRIPT, "--version", "0.1.0", ...args], {
    env: { ...process.env, HOME: home, PATH: `${fakeBin}:${process.env.PATH}`, CLOCKWORK_TEST_RELEASE: release },
    encoding: "utf8",
  });
  return { tmp, home, run };
}

describe("Clockwork release installer", () => {
  it("verifies and installs only the binary by default", () => {
    const x = setup();
    const result = x.run([]);
    assert.strictEqual(result.status, 0, result.stderr);
    assert.match(result.stdout, /Installed Clockwork v0.1.0/);
    assert.ok(fs.existsSync(path.join(x.home, ".local", "bin", "clockwork")));
    assert.ok(!fs.existsSync(path.join(x.home, ".agents", "clockwork")));
    assert.ok(!fs.existsSync(path.join(x.home, "Library", "LaunchAgents")));
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("installs service integration only with the explicit opt-in", () => {
    const x = setup();
    const result = x.run(["--with-service"]);
    assert.strictEqual(result.status, 0, result.stderr);
    assert.ok(fs.existsSync(path.join(x.home, ".agents", "clockwork", "jobs.d")));
    assert.ok(!fs.existsSync(path.join(x.home, ".local", "bin", "clockwork-pi")));
    assert.ok(fs.lstatSync(path.join(x.home, ".local", "bin", "clockwork-service")).isSymbolicLink());
    assert.ok(fs.existsSync(path.join(x.home, "Library", "LaunchAgents", "dev.iurysouza.clockwork.plist")));
    assert.ok(!fs.existsSync(path.join(x.home, ".local", "state", "clockwork", "jobs.json")));
    const rerun = x.run(["--with-service"]);
    assert.strictEqual(rerun.status, 0, rerun.stderr);
    assert.ok(fs.existsSync(path.join(x.home, ".local", "bin", "clockwork-service")));
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("refuses an archive whose checksum does not match", () => {
    const x = setup({ badChecksum: true });
    const result = x.run([]);
    assert.notStrictEqual(result.status, 0);
    assert.match(result.stderr, /checksum mismatch/);
    assert.ok(!fs.existsSync(path.join(x.home, ".local", "bin", "clockwork")));
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });
});
