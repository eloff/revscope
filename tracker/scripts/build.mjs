import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, gzipSync, constants as zlibConstants } from "node:zlib";
import { minify } from "uglify-js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const trackerPath = join(root, "tracker.js");
const fingerprintPath = join(root, "content-fingerprint");
const outputPath = join(root, "tracker-min.js");
const gzipPath = join(root, "tracker-min.js.gz");
const brotliPath = join(root, "tracker-min.js.br");
const versionPattern = /(var version = ")(\d+)\.(\d+)(";)/;
const testBlockPattern = /\n?[ \t]*\/\* RR_TEST_START \*\/[\s\S]*?\/\* RR_TEST_END \*\//g;

function currentVersion(source) {
  const match = source.match(versionPattern);
  if (!match) {
    throw new Error('Could not find var version = "{major}.{minor}"; in tracker.js');
  }
  return `${match[2]}.${match[3]}`;
}

function bumpMinorVersion(source) {
  const match = source.match(versionPattern);
  if (!match) {
    throw new Error('Could not find var version = "{major}.{minor}"; in tracker.js');
  }
  const major = match[2];
  const minor = Number(match[3]) + 1;
  return {
    source: source.replace(versionPattern, `$1${major}.${minor}$4`),
    version: `${major}.${minor}`,
  };
}

function productionSource(source) {
  const stripped = source.replace(testBlockPattern, "");
  if (stripped === source) {
    throw new Error("Could not find RR_TEST_START/RR_TEST_END markers in tracker.js");
  }
  return stripped;
}

function contentFingerprint(source) {
  // Version is excluded so the bump itself does not change the fingerprint.
  const withoutVersion = source.replace(versionPattern, "$1X.Y$4");
  return createHash("sha256").update(productionSource(withoutVersion)).digest("hex");
}

function readLastFingerprint() {
  if (!existsSync(fingerprintPath)) {
    return null;
  }
  return readFileSync(fingerprintPath, "utf8").trim();
}

function verifyEs5(filename) {
  const result = spawnSync(
    "pnpm",
    ["exec", "eslint", "--config", "scripts/eslint-es5-parse.config.mjs", filename],
    { cwd: root, encoding: "utf8" },
  );
  if (result.status !== 0) {
    const output = (result.stdout || result.stderr || "").trim();
    throw new Error(`ES5 eslint check failed for ${filename}:\n${output}`);
  }
}

let source = readFileSync(trackerPath, "utf8");
const fingerprint = contentFingerprint(source);
const lastFingerprint = readLastFingerprint();

if (fingerprint !== lastFingerprint) {
  if (lastFingerprint === null) {
    writeFileSync(fingerprintPath, `${fingerprint}\n`, "utf8");
    console.log(`Seeded tracker content fingerprint; keeping version ${currentVersion(source)}`);
  } else {
    const bumped = bumpMinorVersion(source);
    source = bumped.source;
    writeFileSync(trackerPath, source, "utf8");
    writeFileSync(fingerprintPath, `${fingerprint}\n`, "utf8");
    console.log(`Bumped tracker version to ${bumped.version}`);
  }
} else {
  console.log(`Tracker unchanged; keeping version ${currentVersion(source)}`);
}

// Remove test-only export block before minify so it cannot affect bundle size.
const production = productionSource(source);

const minified = minify(production, {
  compress: { arrows: false },
  mangle: true,
});
if (minified.error) {
  throw minified.error;
}

mkdirSync(dirname(outputPath), { recursive: true });
const minifiedBytes = Buffer.from(minified.code, "utf8");
writeFileSync(outputPath, minifiedBytes);
console.log(`Wrote ${outputPath} (${minifiedBytes.length} bytes)`);

const gzipBytes = gzipSync(minifiedBytes, { level: 9 });
// Header byte 9 records the OS that wrote the file (zlib writes 19 on macOS, 10
// on Windows). Pin it to Unix so the committed output matches CI's rebuild.
gzipBytes[9] = 3;
writeFileSync(gzipPath, gzipBytes);
console.log(`Wrote ${gzipPath} (${gzipBytes.length} bytes)`);

const brotliBytes = brotliCompressSync(minifiedBytes, {
  params: {
    [zlibConstants.BROTLI_PARAM_MODE]: zlibConstants.BROTLI_MODE_TEXT,
    [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
  },
});
writeFileSync(brotliPath, brotliBytes);
console.log(`Wrote ${brotliPath} (${brotliBytes.length} bytes)`);

verifyEs5(outputPath);
console.log("ES5 eslint check passed");
