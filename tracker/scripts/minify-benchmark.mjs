import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import * as esbuild from "esbuild";
import { minify as terserMinify } from "terser";
import { minify as uglifyMinify } from "uglify-js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const inputPath = join(root, "tracker.js");
const outDir = join(root, "scripts", "benchmark-output");
const source = readFileSync(inputPath, "utf8");
const ITERATIONS = 5;

mkdirSync(outDir, { recursive: true });

function sizeInfo(code) {
  const raw = Buffer.byteLength(code, "utf8");
  const gz = gzipSync(code).length;
  return { raw, gz };
}

function eslintEs5Parse(filename) {
  const result = spawnSync(
    "pnpm",
    [
      "exec",
      "eslint",
      "--config",
      "scripts/eslint-es5-parse.config.mjs",
      "--no-error-on-unmatched-pattern",
      filename,
    ],
    { cwd: root, encoding: "utf8" },
  );
  const output = (result.stdout || result.stderr || "").trim();
  return {
    ok: result.status === 0,
    output: output || null,
  };
}

function eslintProjectConfig(filename) {
  const result = spawnSync(
    "pnpm",
    ["exec", "eslint", "--no-error-on-unmatched-pattern", filename],
    { cwd: root, encoding: "utf8" },
  );
  const output = (result.stdout || result.stderr || "").trim();
  const errorCount = (output.match(/\berror\b/g) || []).length;
  return {
    ok: result.status === 0,
    errorCount,
    output: output || null,
  };
}

async function runTerser() {
  const start = performance.now();
  const result = await terserMinify(source, {
    compress: { ecma: 5 },
    mangle: true,
    format: { ecma: 5, comments: false },
  });
  const ms = performance.now() - start;
  if (result.error) throw result.error;
  return { code: result.code, ms };
}

async function runEsbuild() {
  const start = performance.now();
  const result = await esbuild.transform(source, {
    minify: true,
    target: "es5",
    loader: "js",
  });
  const ms = performance.now() - start;
  return { code: result.code, ms };
}

async function runUglify() {
  const start = performance.now();
  const result = uglifyMinify(source, {
    compress: true,
    mangle: true,
  });
  const ms = performance.now() - start;
  if (result.error) throw result.error;
  return { code: result.code, ms };
}

function avg(values) {
  return values.reduce((a, b) => a + b, 0) / values.length;
}

const baseline = sizeInfo(source);
const runners = [
  ["terser", runTerser],
  ["esbuild", runEsbuild],
  ["uglify-js", runUglify],
];

const results = [];

for (const [name, run] of runners) {
  const timings = [];
  let code = "";
  for (let i = 0; i < ITERATIONS; i++) {
    const out = await run();
    timings.push(out.ms);
    code = out.code;
  }

  const outPath = join(outDir, `${name}.min.js`);
  writeFileSync(outPath, code, "utf8");
  const sizes = sizeInfo(code);
  const parseCheck = eslintEs5Parse(outPath);
  const eslint = eslintProjectConfig(outPath);

  results.push({
    name,
    msAvg: Math.round(avg(timings) * 100) / 100,
    msMin: Math.round(Math.min(...timings) * 100) / 100,
    msMax: Math.round(Math.max(...timings) * 100) / 100,
    sizes,
    parseEs5: parseCheck,
    eslint,
    output: outPath,
  });
}

const report = {
  input: {
    path: inputPath,
    ...baseline,
    parseEs5: eslintEs5Parse(inputPath),
    eslint: eslintProjectConfig(inputPath),
  },
  settings: {
    iterations: ITERATIONS,
    terser: "compress.ecma=5, format.ecma=5, mangle",
    esbuild: "minify, target=es5",
    uglifyJs: "compress, mangle (default; no ES5 output option in uglify-js 3.x)",
  },
  results: results.map((r) => ({
    name: r.name,
    msAvg: r.msAvg,
    msMin: r.msMin,
    msMax: r.msMax,
    rawBytes: r.sizes.raw,
    gzipBytes: r.sizes.gz,
    rawReductionPct: Math.round((1 - r.sizes.raw / baseline.raw) * 1000) / 10,
    gzipReductionPct: Math.round((1 - r.sizes.gz / baseline.gz) * 1000) / 10,
    parseEs5Pass: r.parseEs5.ok,
    parseEs5Error: r.parseEs5.output,
    eslintPass: r.eslint.ok,
    eslintErrorCount: r.eslint.errorCount,
    eslintSample: r.eslint.output ? r.eslint.output.split("\n").slice(0, 8).join("\n") : null,
    output: r.output,
  })),
};

console.log(JSON.stringify(report, null, 2));
