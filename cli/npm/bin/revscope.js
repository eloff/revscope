#!/usr/bin/env node
// Runs the native revscope binary, fetching the release for this platform on first
// use. Fetched here rather than in a postinstall script, which npx and agents often
// run with scripts disabled. Everything this prints goes to stderr, since stdout
// belongs to the binary, whose --json output is parsed.
"use strict";

const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { version } = require("../package.json");

const TARGETS = {
  "linux x64": "x86_64-unknown-linux-musl",
  "linux arm64": "aarch64-unknown-linux-musl",
  "darwin x64": "x86_64-apple-darwin",
  "darwin arm64": "aarch64-apple-darwin",
  "win32 x64": "x86_64-pc-windows-msvc",
};

// The binary's own contract, for failures before there is a binary: --json answers on
// stdout, and 75 means try again later.
const EXIT_FAILURE = 1;
const EXIT_RETRY = 75;

function fail(message, exitCode = EXIT_FAILURE) {
  const args = process.argv.slice(2);
  const end = args.indexOf("--");
  if ((end === -1 ? args : args.slice(0, end)).includes("--json")) {
    console.log(JSON.stringify({ error: { message, exit_code: exitCode } }, null, 2));
  } else {
    console.error(`revscope: ${message}`);
  }
  process.exit(exitCode);
}

class NoSuchRelease extends Error {}

function cacheRoot() {
  if (process.env.REVSCOPE_CACHE_DIR) return process.env.REVSCOPE_CACHE_DIR;
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    return path.join(process.env.LOCALAPPDATA, "revscope");
  }
  return path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "revscope");
}

async function download(url) {
  const response = await fetch(url);
  if (response.status === 404) throw new NoSuchRelease(`${url} doesn't exist`);
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

async function fetchBinary(target, binary) {
  const archive = `revscope-${target}${process.platform === "win32" ? ".zip" : ".tar.gz"}`;
  const base =
    process.env.REVSCOPE_RELEASE_URL ||
    `https://github.com/eloff/revscope/releases/download/cli-v${version}`;
  console.error(`revscope: fetching ${archive} for version ${version}`);

  const [bytes, checksum] = await Promise.all([
    download(`${base}/${archive}`),
    download(`${base}/${archive}.sha256`),
  ]);
  const expected = checksum.toString("utf8").trim().split(/\s+/)[0];
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (!expected || expected !== actual) {
    throw new NoSuchRelease(`${archive}'s checksum doesn't match`);
  }

  const directory = path.dirname(binary);
  fs.mkdirSync(directory, { recursive: true });
  const work = fs.mkdtempSync(path.join(directory, ".fetch-"));
  try {
    const archivePath = path.join(work, archive);
    fs.writeFileSync(archivePath, bytes);
    // bsdtar on Windows reads zips too.
    const extracted = spawnSync("tar", ["-xf", archivePath, "-C", work], { stdio: "inherit" });
    if (extracted.status !== 0) throw new Error(`could not extract ${archive}`);
    const unpacked = path.join(work, path.basename(binary));
    fs.chmodSync(unpacked, 0o755);
    // Into place in one step, so a concurrent run never executes half a file.
    fs.renameSync(unpacked, binary);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

async function main() {
  const target = TARGETS[`${process.platform} ${process.arch}`];
  if (!target) fail(`no release for ${process.platform} ${process.arch}`);

  const name = process.platform === "win32" ? "revscope.exe" : "revscope";
  const binary = path.join(cacheRoot(), version, name);
  if (!fs.existsSync(binary)) {
    try {
      await fetchBinary(target, binary);
    } catch (error) {
      // A network failure may pass. A missing or corrupt release won't.
      const exitCode = error instanceof NoSuchRelease ? EXIT_FAILURE : EXIT_RETRY;
      fail(`could not fetch the binary: ${error.message}`, exitCode);
    }
  }

  const run = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
  if (run.error) fail(`could not run ${binary}: ${run.error.message}`);
  if (run.signal) process.kill(process.pid, run.signal);
  process.exit(run.status ?? 1);
}

main();
