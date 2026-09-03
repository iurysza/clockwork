import assert from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { spawnSync } from "node:child_process";

const SCRIPT = new URL("./install.mjs", import.meta.url).pathname;

function setup() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "clockwork-install-"));
  const home = path.join(tmp, "home");
  fs.mkdirSync(home);
  const run = (args) => spawnSync(process.execPath, [SCRIPT, ...args], { env: { ...process.env, HOME: home }, encoding: "utf8" });
  return { tmp, home, run };
}

function installBinary(home) {
  const binary = path.join(home, ".local", "bin", "clockwork");
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(binary, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
}

describe("Clockwork Pi integration installer", () => {
  it("keeps preview zero-write and does not start a service", () => {
    const x = setup();
    const result = x.run([]);
    assert.strictEqual(result.status, 0, result.stderr);
    assert.match(result.stdout, /preview only/);
    assert.deepStrictEqual(fs.readdirSync(x.home), []);
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("requires the separately installed Clockwork binary", () => {
    const x = setup();
    const result = x.run(["--apply"]);
    assert.notStrictEqual(result.status, 0);
    assert.match(result.stderr, /Install Clockwork before adding the Pi integration/);
    assert.deepStrictEqual(fs.readdirSync(x.home), []);
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("installs managed integration files without loading launchd or applying jobs", () => {
    const x = setup();
    installBinary(x.home);
    const obsolete = path.join(x.home, ".local", "bin", "clockwork-jobs");
    fs.symlinkSync(path.join(path.dirname(SCRIPT), "services", "clockwork", "clockwork-jobs.mjs"), obsolete);
    const result = x.run(["--apply"]);
    assert.strictEqual(result.status, 0, result.stderr);
    const jobs = path.join(x.home, ".agents", "clockwork", "jobs.d");
    const state = path.join(x.home, ".local", "state", "clockwork");
    const plist = path.join(x.home, "Library", "LaunchAgents", "dev.iurysouza.clockwork.plist");
    assert.ok(fs.existsSync(jobs));
    assert.deepStrictEqual(fs.readdirSync(jobs), []);
    assert.ok(fs.existsSync(plist));
    assert.ok(fs.lstatSync(path.join(x.home, ".local", "bin", "clockwork-service")).isSymbolicLink());
    assert.throws(() => fs.lstatSync(obsolete), { code: "ENOENT" });
    assert.strictEqual(fs.statSync(state).mode & 0o077, 0);
    assert.ok(!fs.existsSync(path.join(state, "jobs.json")));
    assert.ok(!fs.existsSync(path.join(state, "daemon.pid")));
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("preserves a user-owned command at the obsolete wrapper path", () => {
    const x = setup();
    installBinary(x.home);
    const command = path.join(x.home, ".local", "bin", "clockwork-jobs");
    fs.writeFileSync(command, "user owned\n");
    const result = x.run(["--apply"]);
    assert.strictEqual(result.status, 0, result.stderr);
    assert.strictEqual(fs.readFileSync(command, "utf8"), "user owned\n");
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });

  it("runs doctor without writing files or loading launchd", () => {
    const x = setup();
    const result = x.run(["--doctor"]);
    assert.strictEqual(result.status, 0, result.stderr);
    assert.match(result.stdout, /Clockwork plist rendering/);
    assert.deepStrictEqual(fs.readdirSync(x.home), []);
    fs.rmSync(x.tmp, { recursive: true, force: true });
  });
});
