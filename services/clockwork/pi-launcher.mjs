#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const THINKING = new Set(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);
const PROFILE_KEYS = new Set(["version", "cwd", "model", "thinking", "tools", "approveProjectFiles"]);

export function expandHome(value, home = os.homedir()) {
  return value === "~" ? home : value.startsWith("~/") ? path.join(home, value.slice(2)) : value;
}

export function validatePiProfile(value, { home = os.homedir(), requireCwd = true } = {}) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("pi-profile.json must contain an object");
  const extra = Object.keys(value).filter((key) => !PROFILE_KEYS.has(key));
  if (extra.length) throw new Error(`pi-profile.json contains unsupported keys: ${extra.join(", ")}`);
  if (value.version !== 1) throw new Error("pi-profile.json version must be 1");
  if (typeof value.cwd !== "string" || !value.cwd) throw new Error("pi-profile.json cwd is required");
  if (typeof value.model !== "string" || !/^[^/\s]+\/[^/\s]+$/.test(value.model)) throw new Error("pi-profile.json model must be provider/model");
  if (!THINKING.has(value.thinking)) throw new Error("pi-profile.json thinking is invalid");
  if (!Array.isArray(value.tools) || value.tools.length === 0 || value.tools.some((tool) => typeof tool !== "string" || !/^[a-z][a-z0-9_-]*$/.test(tool))) throw new Error("pi-profile.json tools must be a non-empty safe string array");
  if (new Set(value.tools).size !== value.tools.length) throw new Error("pi-profile.json tools must not contain duplicates");
  if (typeof value.approveProjectFiles !== "boolean") throw new Error("pi-profile.json approveProjectFiles must be boolean");
  const cwd = path.resolve(expandHome(value.cwd, home));
  if (requireCwd && (!fs.existsSync(cwd) || !fs.statSync(cwd).isDirectory())) throw new Error(`pi-profile.json cwd is not a directory: ${cwd}`);
  return { ...value, cwd };
}

export function launcherArgs(job, profile, { home = os.homedir() } = {}) {
  if (!SAFE_NAME.test(job)) throw new Error("invalid job identity");
  const sessionDir = path.join(process.env.CLOCKWORK_HOME || path.join(home, ".local/state/clockwork"), "pi-sessions", job);
  const args = ["--print", "--mode", "json", "--session-id", `clockwork-${job}`, "--session-dir", sessionDir, "--model", profile.model, "--thinking", profile.thinking, "--tools", profile.tools.join(",")];
  args.push(profile.approveProjectFiles ? "--approve" : "--no-approve");
  return { args, sessionDir };
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.length !== 2 || argv[0] !== "--job" || !SAFE_NAME.test(argv[1])) throw new Error("usage: clockwork-pi --job <managed-job-name>");
  const job = argv[1];
  const home = os.homedir();
  const jobsRoot = process.env.CLOCKWORK_JOBS_ROOT || path.join(home, ".agents/clockwork/jobs.d");
  const profile = validatePiProfile(JSON.parse(fs.readFileSync(path.join(jobsRoot, job, "pi-profile.json"), "utf8")), { home });
  const pi = process.env.PI_BIN || path.join(home, ".pi/agent/bin/pi");
  if (!fs.existsSync(pi)) throw new Error(`managed Pi binary missing: ${pi}`);
  const { args, sessionDir } = launcherArgs(job, profile, { home });
  fs.mkdirSync(sessionDir, { recursive: true, mode: 0o700 });
  fs.chmodSync(sessionDir, 0o700);
  process.chdir(profile.cwd);
  const child = spawn(pi, args, { stdio: ["inherit", "inherit", "inherit"], env: process.env });
  child.on("error", (error) => { throw error; });
  child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 1)));
}

const invokedPath = process.argv[1] && fs.realpathSync(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error.message); process.exit(2); });
