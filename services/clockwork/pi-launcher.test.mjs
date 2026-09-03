import assert from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { spawnSync } from "node:child_process";
import { launcherArgs, validatePiProfile } from "./pi-launcher.mjs";

const SCRIPT = new URL("./pi-launcher.mjs", import.meta.url).pathname;

describe("clockwork Pi launcher", () => {
  it("rejects raw or unknown profile fields", () => {
    assert.throws(() => validatePiProfile({ version: 1, cwd: "/tmp", model: "p/m", thinking: "high", tools: ["read"], approveProjectFiles: true, args: ["--session-id", "escape"] }, { requireCwd: false }), /unsupported keys/);
  });

  it("derives stable isolated sessions and forwards bounded settings", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "clockwork-pi-"));
    const home = path.join(tmp, "home"); const cwd = path.join(tmp, "project"); const jobs = path.join(home, ".agents/clockwork/jobs.d");
    fs.mkdirSync(cwd, { recursive: true }); fs.mkdirSync(path.join(jobs, "alpha"), { recursive: true }); fs.mkdirSync(path.join(jobs, "beta"), { recursive: true });
    const profile = { version: 1, cwd, model: "provider/model", thinking: "xhigh", tools: ["read", "bash", "write"], approveProjectFiles: true };
    for (const job of ["alpha", "beta"]) fs.writeFileSync(path.join(jobs, job, "pi-profile.json"), JSON.stringify(profile));
    const capture = path.join(tmp, "capture.json"); const fake = path.join(tmp, "pi");
    fs.writeFileSync(fake, `#!/usr/bin/env node\nconst fs=require('fs');let s='';process.stdin.on('data',d=>s+=d).on('end',()=>fs.writeFileSync(process.env.CAPTURE,JSON.stringify({args:process.argv.slice(2),cwd:process.cwd(),stdin:s})));`); fs.chmodSync(fake, 0o755);
    const run = (job) => spawnSync(process.execPath, [SCRIPT, "--job", job], { input: "harmless prompt", env: { ...process.env, HOME: home, CLOCKWORK_JOBS_ROOT: jobs, CLOCKWORK_HOME: path.join(home, ".local/state/clockwork"), PI_BIN: fake, CAPTURE: capture }, encoding: "utf8" });
    assert.strictEqual(run("alpha").status, 0); const first = JSON.parse(fs.readFileSync(capture));
    assert.deepStrictEqual(first.args, ["--print", "--mode", "json", "--session-id", "clockwork-alpha", "--session-dir", path.join(home, ".local/state/clockwork/pi-sessions/alpha"), "--model", "provider/model", "--thinking", "xhigh", "--tools", "read,bash,write", "--approve"]);
    assert.strictEqual(fs.realpathSync(first.cwd), fs.realpathSync(cwd)); assert.strictEqual(first.stdin, "harmless prompt");
    assert.strictEqual(run("alpha").status, 0); assert.deepStrictEqual(JSON.parse(fs.readFileSync(capture)).args, first.args);
    assert.strictEqual(run("beta").status, 0); assert.match(JSON.parse(fs.readFileSync(capture)).args.join(" "), /clockwork-beta/);
    assert.notDeepStrictEqual(launcherArgs("alpha", profile, { home }).sessionDir, launcherArgs("beta", profile, { home }).sessionDir);
    const installed = path.join(tmp, "clockwork-pi"); fs.symlinkSync(SCRIPT, installed);
    const linked = spawnSync(installed, ["--job", "alpha"], { input: "harmless prompt", env: { ...process.env, HOME: home, CLOCKWORK_JOBS_ROOT: jobs, CLOCKWORK_HOME: path.join(home, ".local/state/clockwork"), PI_BIN: fake, CAPTURE: capture }, encoding: "utf8" });
    assert.strictEqual(linked.status, 0, linked.stderr); assert.match(JSON.parse(fs.readFileSync(capture)).args.join(" "), /clockwork-alpha/);
    fs.rmSync(tmp, { recursive: true, force: true });
  });
});
