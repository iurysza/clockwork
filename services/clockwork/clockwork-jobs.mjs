#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { validatePiProfile } from "./pi-launcher.mjs";

const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const COMMANDS = new Set(["check", "plan", "apply", "status"]);
const EXIT = { EXECUTION: 1, INPUT: 2, ABSENT: 3 };

function atomicJson(target, value) {
  fs.mkdirSync(path.dirname(target), { recursive: true, mode: 0o700 });
  const tmp = `${target}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(tmp, target);
}

function runClockwork(bin, args, env, { allowFailure = false } = {}) {
  const result = spawnSync(bin, args, { env, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) throw new Error(`clockwork ${args.slice(0, 2).join(" ")} failed: ${(result.stderr || result.stdout).trim()}`);
  return result;
}

function jsonResult(result, fallback = []) {
  if (result.status !== 0 || !result.stdout.trim()) return fallback;
  try { return JSON.parse(result.stdout); } catch { throw new Error("clockwork returned invalid JSON"); }
}

function previewManifest(clockwork, file, env, clockworkHome) {
  const shadow = fs.mkdtempSync(path.join(os.tmpdir(), "clockwork-plan-"));
  try {
    for (const relative of ["jobs.json", "config.json", "manifests"]) {
      const source = path.join(clockworkHome, relative);
      if (fs.existsSync(source)) fs.cpSync(source, path.join(shadow, relative), { recursive: true });
    }
    return jsonResult(runClockwork(clockwork, ["up", "--file", file, "--dry-run", "--json"], { ...env, CLOCKWORK_HOME: shadow }), {});
  } finally {
    fs.rmSync(shadow, { recursive: true, force: true });
  }
}

export function inspectManifest(text, directoryName) {
  if (!SAFE_NAME.test(directoryName)) throw new Error(`invalid job directory name: ${directoryName}`);
  if (/^\s*allow_insecure_http\s*:\s*true\s*$/m.test(text)) throw new Error(`${directoryName}: insecure HTTP is forbidden`);
  const name = text.match(/^name:\s*["']?([^"'\s#]+)["']?\s*(?:#.*)?$/m)?.[1];
  if (name !== directoryName) throw new Error(`${directoryName}: manifest name must equal directory name`);
  const jobsLine = text.match(/^jobs:\s*(?:#.*)?$/m);
  if (!jobsLine) throw new Error(`${directoryName}: manifest must contain jobs`);
  const after = text.slice((jobsLine.index ?? 0) + jobsLine[0].length);
  const keys = [...after.matchAll(/^ {2}([A-Za-z0-9][A-Za-z0-9._-]{0,63}):\s*(?:#.*)?$/gm)].map((m) => m[1]);
  if (keys.length !== 1 || keys[0] !== directoryName) throw new Error(`${directoryName}: jobs must contain exactly one matching key`);
  const hasRun = /^ {4}run:\s*.+$/m.test(after);
  const hasPrompt = /^ {4}prompt:\s*(?:.+|[>|]-?)$/m.test(after);
  const webhook = after.match(/^ {4}webhook:\s*["']?([^"'\s#]+)["']?/m)?.[1];
  const actionCount = Number(hasRun) + Number(hasPrompt) + Number(Boolean(webhook));
  if (actionCount !== 1) throw new Error(`${directoryName}: job must declare exactly one action`);
  if (webhook && !webhook.startsWith("https://")) throw new Error(`${directoryName}: webhook must use HTTPS`);
  const pausedMatch = after.match(/^ {4}paused:\s*(true|false)\s*(?:#.*)?$/m);
  const agent = after.match(/^ {4}agent:\s*["']?([^"'\s#]+)["']?/m)?.[1];
  if (hasPrompt && agent !== `clockwork-pi-${directoryName}`) throw new Error(`${directoryName}: prompt agent must be clockwork-pi-${directoryName}`);
  return { name, action: hasPrompt ? "prompt" : webhook ? "webhook" : "command", paused: pausedMatch ? pausedMatch[1] === "true" : null, agent, webhook };
}

export function loadSources(jobsRoot, selected) {
  if (!fs.existsSync(jobsRoot)) return [];
  const dirs = fs.readdirSync(jobsRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort();
  if (selected && !dirs.includes(selected)) { const error = new Error(`job not found: ${selected}`); error.exitCode = EXIT.ABSENT; throw error; }
  const wanted = selected ? dirs.filter((name) => name === selected) : dirs;
  return wanted.map((name) => {
    const dir = path.join(jobsRoot, name);
    const files = fs.readdirSync(dir).filter((file) => !file.startsWith("."));
    if (!files.includes("clockwork.yaml")) throw new Error(`${name}: clockwork.yaml is required`);
    const manifestPath = path.join(dir, "clockwork.yaml");
    const manifest = inspectManifest(fs.readFileSync(manifestPath, "utf8"), name);
    const profilePath = path.join(dir, "pi-profile.json");
    let piProfile = null;
    if (manifest.action === "prompt") {
      if (!fs.existsSync(profilePath)) throw new Error(`${name}: pi-profile.json is required for prompt jobs`);
      piProfile = validatePiProfile(JSON.parse(fs.readFileSync(profilePath, "utf8")));
      const allowed = new Set(["clockwork.yaml", "pi-profile.json"]);
      if (files.some((file) => !allowed.has(file))) throw new Error(`${name}: unsupported source file`);
    } else if (fs.existsSync(profilePath)) throw new Error(`${name}: pi-profile.json is only allowed for prompt jobs`);
    else if (files.some((file) => file !== "clockwork.yaml")) throw new Error(`${name}: unsupported source file`);
    return { name, dir, manifestPath, manifest, piProfile };
  });
}

function parseArgs(argv) {
  const command = argv.shift();
  if (!COMMANDS.has(command)) throw new Error("usage: clockwork-jobs <check|plan|apply|status> [job] [--json] [--confirm <job|all>] [--no-input]");
  let selected = null; let json = false; let confirm = null;
  while (argv.length) {
    const arg = argv.shift();
    if (arg === "--json") json = true;
    else if (arg === "--no-input") { /* explicitly non-interactive; confirmation still required */ }
    else if (arg === "--confirm") confirm = argv.shift();
    else if (!arg.startsWith("-") && !selected) selected = arg;
    else throw new Error(`unsupported argument: ${arg}`);
  }
  if (command === "apply" && confirm !== (selected || "all")) throw new Error(`apply requires --confirm ${selected || "all"}`);
  if (command !== "apply" && confirm) throw new Error("--confirm is only valid for apply");
  return { command, selected, json, confirm };
}

function managedState(stateFile) {
  if (!fs.existsSync(stateFile)) return { version: 1, jobs: {} };
  const value = JSON.parse(fs.readFileSync(stateFile, "utf8"));
  if (value.version !== 1 || !value.jobs || typeof value.jobs !== "object") throw new Error("invalid integration ownership state");
  return value;
}

function desiredProfile(source, launcher) {
  return { name: `clockwork-pi-${source.name}`, bin: launcher, args: ["--job", source.name], prompt_stdin: true };
}

function normalizedProfiles(value) { return Array.isArray(value) ? value : value.agents || []; }
function normalizedJobs(value) { return Array.isArray(value) ? value : value.jobs || []; }

function makePlan({ sources, ownership, profiles, runtimeJobs, launcher, selected }) {
  const actions = [];
  const runtimeByManifest = new Map();
  for (const job of runtimeJobs) {
    const manifest = job.managed_by || job.manifest || job.manifest_name || job.source_manifest;
    if (manifest) runtimeByManifest.set(manifest, job);
  }
  const profileByName = new Map(profiles.map((profile) => [profile.name, profile]));
  for (const source of sources) {
    const owned = ownership.jobs[source.name];
    const collision = runtimeByManifest.get(source.name);
    if (collision && !owned) throw new Error(`${source.name}: unmanaged manifest collision`);
    if (!owned && source.manifest.paused !== true) throw new Error(`${source.name}: first apply must set paused: true`);
    if (source.piProfile) {
      const desired = desiredProfile(source, launcher);
      const current = profileByName.get(desired.name);
      if (current && !owned?.profile) throw new Error(`${source.name}: unmanaged agent profile collision`);
      if (!current || current.bin !== desired.bin || JSON.stringify(current.args) !== JSON.stringify(desired.args) || current.prompt_stdin !== true) actions.push({ type: "profile-upsert", job: source.name, profile: desired });
    }
    actions.push({ type: "manifest-up", job: source.name, file: source.manifestPath, paused: source.manifest.paused });
  }
  if (!selected) {
    const present = new Set(sources.map((source) => source.name));
    for (const [name, owned] of Object.entries(ownership.jobs).sort()) if (!present.has(name)) actions.push({ type: "manifest-down", job: name, manifest: owned.manifest, profile: owned.profile || null });
  }
  return actions;
}

function output(value, json) {
  if (json) process.stdout.write(`${JSON.stringify(value)}\n`);
  else if (value.actions) for (const action of value.actions) console.log(`${action.type}: ${action.job}`);
  else console.log(value.message || "ok");
}

async function main() {
  let options;
  try { options = parseArgs(process.argv.slice(2)); } catch (error) { error.exitCode = EXIT.INPUT; throw error; }
  const home = os.homedir();
  const clockworkHome = process.env.CLOCKWORK_HOME || path.join(home, ".local/state/clockwork");
  const jobsRoot = process.env.CLOCKWORK_JOBS_ROOT || path.join(home, ".agents/clockwork/jobs.d");
  const clockwork = process.env.CLOCKWORK_BIN || path.join(home, ".local/bin/clockwork");
  const launcher = process.env.CLOCKWORK_PI_BIN || path.join(home, ".local/bin/clockwork-pi");
  const stateFile = path.join(clockworkHome, "integration/ownership.json");
  const env = { ...process.env, CLOCKWORK_HOME: clockworkHome, CLOCKWORK_BACKEND: "none" };
  let sources;
  try { sources = loadSources(jobsRoot, options.selected); } catch (error) { if (!error.exitCode) error.exitCode = EXIT.INPUT; throw error; }
  if (options.command === "check") return output({ ok: true, jobs: sources.map((source) => source.name), message: `${sources.length} job(s) valid` }, options.json);
  const ownership = managedState(stateFile);
  const profiles = normalizedProfiles(jsonResult(runClockwork(clockwork, ["agent", "list", "--json"], env), []));
  const runtimeJobs = normalizedJobs(jsonResult(runClockwork(clockwork, ["list", "--json"], env), []));
  if (options.command === "status") return output({ jobs: sources.map((source) => ({ name: source.name, managed: Boolean(ownership.jobs[source.name]), runtime: runtimeJobs.filter((job) => (job.managed_by || job.manifest || job.manifest_name || job.source_manifest) === source.name) })) }, options.json);
  const actions = makePlan({ sources, ownership, profiles, runtimeJobs, launcher, selected: options.selected });
  for (const action of actions.filter((item) => item.type === "manifest-up")) action.preview = previewManifest(clockwork, action.file, env, clockworkHome);
  if (options.command === "plan") return output({ actions }, options.json);
  const next = structuredClone(ownership);
  for (const action of actions.filter((item) => item.type === "profile-upsert")) {
    const args = ["agent", "add", "--bin", action.profile.bin, "--prompt-stdin"];
    for (const arg of action.profile.args) args.push(`--arg=${arg}`);
    args.push(action.profile.name);
    runClockwork(clockwork, args, env);
  }
  for (const source of sources) {
    runClockwork(clockwork, ["up", "--file", source.manifestPath], env);
    next.jobs[source.name] = { source: source.dir, manifest: source.name, profile: source.piProfile ? `clockwork-pi-${source.name}` : null };
  }
  for (const action of actions.filter((item) => item.type === "manifest-down")) {
    runClockwork(clockwork, ["down", "--manifest", action.manifest], env);
    if (action.profile) runClockwork(clockwork, ["agent", "rm", action.profile], env);
    delete next.jobs[action.job];
  }
  atomicJson(stateFile, next);
  output({ applied: true, actions }, options.json);
}

const invokedPath = process.argv[1] && fs.realpathSync(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error.message); process.exit(error.exitCode || EXIT.EXECUTION); });
