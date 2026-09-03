#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ALLOWED_ACCOUNT = "iurysza@gmail.com";
const KEYS = new Set(["--account", "--date", "--subject", "--html-file"]);

export function berlinDate(now = new Date()) {
  const parts = new Intl.DateTimeFormat("en-CA", { timeZone: "Europe/Berlin", year: "numeric", month: "2-digit", day: "2-digit" }).formatToParts(now);
  const get = (type) => parts.find((part) => part.type === type)?.value;
  return `${get("year")}-${get("month")}-${get("day")}`;
}

function parseArgs(argv) {
  const values = {};
  while (argv.length) {
    const key = argv.shift();
    if (!KEYS.has(key) || Object.hasOwn(values, key)) throw new Error(`unsupported or duplicate argument: ${key}`);
    const value = argv.shift();
    if (!value || value.startsWith("--")) throw new Error(`${key} requires a value`);
    values[key] = value;
  }
  for (const key of KEYS) if (!values[key]) throw new Error(`${key} is required`);
  if (values["--account"] !== ALLOWED_ACCOUNT) throw new Error(`account must be ${ALLOWED_ACCOUNT}`);
  if (values["--date"] !== berlinDate()) throw new Error("date must equal the current Europe/Berlin date");
  if (!values["--subject"].includes(values["--date"])) throw new Error("subject must contain the Berlin date");
  const html = path.resolve(values["--html-file"]);
  if (!fs.existsSync(html) || !fs.statSync(html).isFile()) throw new Error("html file does not exist");
  return { account: values["--account"], date: values["--date"], subject: values["--subject"], html };
}

function atomicReceipt(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const safe = { status: value.status, date: value.date, account: value.account, subject: value.subject, ...(value.messageId ? { messageId: value.messageId } : {}), attemptedAt: value.attemptedAt, updatedAt: new Date().toISOString() };
  const tmp = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, `${JSON.stringify(safe, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(tmp, file);
  return safe;
}

function runGog(gog, args) {
  return spawnSync(gog, args, { encoding: "utf8", timeout: Number(process.env.CLOCKWORK_GOG_TIMEOUT_MS || 120000), maxBuffer: 10 * 1024 * 1024 });
}

function parseJson(result) {
  if (result.error || result.status !== 0 || !result.stdout?.trim()) return null;
  try { return JSON.parse(result.stdout); } catch { return null; }
}

function messageIds(value) {
  const items = Array.isArray(value) ? value : value?.messages || value?.results || value?.data?.messages || [];
  return items.map((item) => typeof item === "string" ? item : item?.id || item?.messageId).filter(Boolean);
}

function sendMessageId(value) { return value?.id || value?.messageId || value?.message?.id || value?.data?.id || value?.data?.messageId || null; }

function headersFrom(value) {
  const source = value?.payload?.headers || value?.headers || value?.message?.payload?.headers || value?.data?.payload?.headers || [];
  if (Array.isArray(source)) return Object.fromEntries(source.map((header) => [String(header.name || "").toLowerCase(), String(header.value || "")]));
  return Object.fromEntries(Object.entries(source).map(([key, val]) => [key.toLowerCase(), String(val)]));
}

function metadataDate(value) {
  const raw = value?.internalDate || value?.date || value?.message?.internalDate || value?.data?.internalDate;
  if (!raw) return null;
  const date = /^\d+$/.test(String(raw)) ? new Date(Number(raw)) : new Date(raw);
  return Number.isNaN(date.valueOf()) ? null : berlinDate(date);
}

function verifyMessage(gog, id, expected) {
  const result = runGog(gog, ["gmail", "get", id, "--account", expected.account, "--format", "metadata", "--json", "--no-input"]);
  const value = parseJson(result);
  if (!value) return false;
  const headers = headersFrom(value);
  const normalize = (text) => text.replace(/^.*<([^>]+)>.*$/, "$1").trim().toLowerCase();
  return normalize(headers.from || "") === expected.account && normalize(headers.to || "") === expected.account && headers.subject === expected.subject && metadataDate(value) === expected.date;
}

function search(gog, expected) {
  const query = `in:sent from:${expected.account} to:${expected.account} subject:"${expected.subject.replaceAll('"', "")}"`;
  const result = runGog(gog, ["gmail", "search", query, "--account", expected.account, "--json", "--no-input"]);
  const parsed = parseJson(result);
  if (!parsed) throw new Error("Sent search failed or returned unknown JSON");
  return messageIds(parsed).filter((id) => verifyMessage(gog, id, expected));
}

export function receiptBlocksRetry(receipt) { return ["attempting", "ambiguous", "delivered"].includes(receipt?.status); }

async function main() {
  const expected = parseArgs(process.argv.slice(2));
  const stateRoot = process.env.CLOCKWORK_HOME || path.join(os.homedir(), ".local/state/clockwork");
  const dir = path.join(stateRoot, "delivery-receipts", "daily-brief-personal");
  const receiptFile = path.join(dir, `${expected.date}.json`);
  const lockDir = path.join(stateRoot, "locks", `daily-brief-personal-${expected.date}.lock`);
  fs.mkdirSync(path.dirname(lockDir), { recursive: true, mode: 0o700 });
  try { fs.mkdirSync(lockDir, { mode: 0o700 }); } catch (error) { if (error.code === "EEXIST") throw new Error("delivery guard is already locked for this date"); throw error; }
  const gog = process.env.GOG_BIN || "gog";
  try {
    if (fs.existsSync(receiptFile)) {
      const receipt = JSON.parse(fs.readFileSync(receiptFile, "utf8"));
      if (receipt.status === "delivered") { console.log(JSON.stringify({ status: "already-delivered", messageId: receipt.messageId })); return; }
      if (receiptBlocksRetry(receipt)) throw new Error(`automatic retry blocked by ${receipt.status} receipt`);
    }
    let matches;
    try { matches = search(gog, expected); } catch (error) { atomicReceipt(receiptFile, { ...expected, status: "ambiguous" }); throw error; }
    if (matches.length === 1) { atomicReceipt(receiptFile, { ...expected, status: "delivered", messageId: matches[0] }); console.log(JSON.stringify({ status: "already-delivered", messageId: matches[0] })); return; }
    if (matches.length > 1) { atomicReceipt(receiptFile, { ...expected, status: "ambiguous" }); throw new Error("multiple matching Sent messages; automatic delivery blocked"); }
    const attemptedAt = new Date().toISOString();
    atomicReceipt(receiptFile, { ...expected, status: "attempting", attemptedAt });
    const sent = runGog(gog, ["gmail", "send", "--account", expected.account, "--to", expected.account, "--subject", expected.subject, "--body-html-file", expected.html, "--json", "--no-input"]);
    const parsed = parseJson(sent);
    let id = sendMessageId(parsed);
    let verified = id ? verifyMessage(gog, id, expected) : false;
    if (!verified) {
      try { const reconciled = search(gog, expected); if (reconciled.length === 1) { id = reconciled[0]; verified = true; } } catch { /* ambiguity below */ }
    }
    if (!verified) { atomicReceipt(receiptFile, { ...expected, status: "ambiguous", messageId: id, attemptedAt }); throw new Error("send outcome is ambiguous; automatic retry blocked"); }
    atomicReceipt(receiptFile, { ...expected, status: "delivered", messageId: id, attemptedAt });
    console.log(JSON.stringify({ status: "delivered", messageId: id }));
  } finally { fs.rmSync(lockDir, { recursive: true, force: true }); }
}

const invokedPath = process.argv[1] && fs.realpathSync(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error.message); process.exit(1); });
