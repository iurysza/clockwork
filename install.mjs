#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const REPO_ROOT = path.dirname(fileURLToPath(import.meta.url));
const SERVICE_DIR = path.join(REPO_ROOT, "services", "clockwork");

function usage() {
  console.log("Usage: node install.mjs [--apply] [--doctor]");
}

function parseArgs(argv) {
  const options = { apply: false, doctor: false };
  for (const arg of argv) {
    if (arg === "--apply") options.apply = true;
    else if (arg === "--doctor") options.doctor = true;
    else if (arg === "--help") { usage(); process.exit(0); }
    else throw new Error(`unsupported argument: ${arg}`);
  }
  if (options.apply && options.doctor) throw new Error("--apply and --doctor cannot be used together");
  return options;
}

function paths(home) {
  const stateDir = path.join(home, ".local", "state", "clockwork");
  const configDir = path.join(home, ".agents", "clockwork");
  const label = "dev.iurysouza.clockwork";
  return {
    home,
    binary: path.join(home, ".local", "bin", "clockwork"),
    configDir,
    envFile: path.join(configDir, "env"),
    jobsDir: path.join(configDir, "jobs.d"),
    label,
    plist: path.join(home, "Library", "LaunchAgents", `${label}.plist`),
    stateDir,
  };
}

function writeAtomic(file, content, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const temporary = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(temporary, content, { mode });
  fs.renameSync(temporary, file);
  fs.chmodSync(file, mode);
}

function renderPlist(target) {
  const template = fs.readFileSync(path.join(SERVICE_DIR, "templates", "dev.iurysouza.clockwork.plist.tpl"), "utf8");
  return template
    .replaceAll("{{SERVICE_DIR}}", SERVICE_DIR)
    .replaceAll("{{ENV_FILE}}", target.envFile)
    .replaceAll("{{STATE_DIR}}", target.stateDir)
    .replaceAll("{{LABEL}}", target.label);
}

function lintPlist(text) {
  const result = spawnSync("plutil", ["-lint", "-"], { input: text, encoding: "utf8" });
  if (result.error) throw new Error(`plutil is required: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`rendered plist is invalid: ${(result.stderr || result.stdout).trim()}`);
}

function linkManaged(source, destination, releaseRoot) {
  try {
    const stat = fs.lstatSync(destination);
    if (!stat.isSymbolicLink()) throw new Error(`refusing to replace non-managed command: ${destination}`);
    const current = fs.realpathSync(destination);
    if (current === fs.realpathSync(source)) return;
    if (!current.startsWith(`${releaseRoot}${path.sep}`)) {
      throw new Error(`refusing to replace non-managed command: ${destination}`);
    }
    fs.unlinkSync(destination);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.symlinkSync(source, destination);
}

function removeObsoleteManagedLink(destination, releaseRoot) {
  let stat;
  try {
    stat = fs.lstatSync(destination);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  if (!stat.isSymbolicLink()) return;

  const linked = path.resolve(path.dirname(destination), fs.readlinkSync(destination));
  const checkoutTarget = path.join(SERVICE_DIR, "clockwork-jobs.mjs");
  if (linked === checkoutTarget || linked.startsWith(`${releaseRoot}${path.sep}`)) {
    fs.unlinkSync(destination);
  }
}

function installIntegration(target) {
  if (!fs.existsSync(target.binary)) {
    throw new Error(`Clockwork binary missing: ${target.binary}. Install Clockwork before adding the Pi integration.`);
  }

  fs.mkdirSync(target.configDir, { recursive: true, mode: 0o700 });
  fs.chmodSync(target.configDir, 0o700);
  fs.mkdirSync(target.jobsDir, { recursive: true, mode: 0o700 });
  fs.chmodSync(target.jobsDir, 0o700);
  for (const relative of ["", "logs", "locks"]) {
    const directory = path.join(target.stateDir, relative);
    fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    fs.chmodSync(directory, 0o700);
  }
  if (!fs.existsSync(target.envFile)) {
    fs.copyFileSync(path.join(SERVICE_DIR, "templates", "env.example"), target.envFile, fs.constants.COPYFILE_EXCL);
    fs.chmodSync(target.envFile, 0o600);
  }
  writeAtomic(target.plist, renderPlist(target), 0o644);
  const binDir = path.dirname(target.binary);
  const releaseRoot = path.join(process.env.XDG_DATA_HOME || path.join(target.home, ".local", "share"), "clockwork", "releases");
  removeObsoleteManagedLink(path.join(binDir, "clockwork-jobs"), releaseRoot);
  linkManaged(path.join(SERVICE_DIR, "pi-launcher.mjs"), path.join(binDir, "clockwork-pi"), releaseRoot);
  linkManaged(path.join(SERVICE_DIR, "service.sh"), path.join(binDir, "clockwork-service"), releaseRoot);
}

function doctor(target) {
  let failed = false;
  const check = (label, command, args, options = {}) => {
    const result = spawnSync(command, args, { encoding: "utf8", ...options });
    if (result.error || result.status !== 0) {
      console.log(`fail    ${label}`);
      failed = true;
    } else console.log(`ok      ${label}`);
  };
  check("Clockwork Pi launcher syntax", process.execPath, ["--check", path.join(SERVICE_DIR, "pi-launcher.mjs")]);
  check("Clockwork service shell syntax", "bash", ["-n", path.join(SERVICE_DIR, "service.sh")]);
  check("Clockwork launchd runner syntax", "zsh", ["-n", path.join(SERVICE_DIR, "launchd-run.sh")]);
  try { lintPlist(renderPlist(target)); console.log("ok      Clockwork plist rendering"); } catch { console.log("fail    Clockwork plist rendering"); failed = true; }
  return failed ? 1 : 0;
}

export function main(argv = process.argv.slice(2), home = os.homedir()) {
  const options = parseArgs(argv);
  const target = paths(home);
  if (options.doctor) return doctor(target);

  console.log("Clockwork Pi integration");
  console.log(`state: ${target.stateDir}`);
  if (!options.apply) {
    console.log("preview only: no files, jobs, services, or external effects will change");
    return 0;
  }

  installIntegration(target);
  console.log("installed Pi integration files only; launchd remains unloaded and jobs remain untouched");
  return 0;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
