// Runs the npm wrapper and install.sh against a fake release server, with a fake
// binary in place of the real one, so they're tested without a published release.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { createServer } from "node:http";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const wrapper = path.join(here, "../bin/revscope.js");
const installer = path.join(here, "../../install.sh");
const targets = {
  "linux x64": "x86_64-unknown-linux-musl",
  "linux arm64": "aarch64-unknown-linux-musl",
  "darwin x64": "x86_64-apple-darwin",
  "darwin arm64": "aarch64-apple-darwin",
};
const target = targets[`${process.platform} ${process.arch}`];

let root;
let server;
let baseUrl;
let files;
let requests;

before(async () => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "revscope-installers-"));
  const stage = path.join(root, "stage");
  fs.mkdirSync(stage);
  // Prints its arguments and exits with a status they name, to show both pass through.
  fs.writeFileSync(
    path.join(stage, "revscope"),
    '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "revscope 9.9.9"; exit 0; fi\necho "ran: $*"\nexit "${FAKE_EXIT:-0}"\n',
    { mode: 0o755 },
  );
  const archive = `revscope-${target}.tar.gz`;
  const tarball = path.join(root, archive);
  const packed = spawnSync("tar", ["-czf", tarball, "-C", stage, "revscope"]);
  assert.equal(packed.status, 0);
  const bytes = fs.readFileSync(tarball);
  const digest = createHash("sha256").update(bytes).digest("hex");
  files = {
    [`/${archive}`]: bytes,
    [`/${archive}.sha256`]: Buffer.from(`${digest}  ${archive}\n`),
  };
  requests = [];
  server = createServer((request, response) => {
    requests.push(request.url);
    const body = files[request.url];
    response.writeHead(body ? 200 : 404);
    response.end(body);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  baseUrl = `http://127.0.0.1:${server.address().port}`;
});

after(() => {
  server.close();
  fs.rmSync(root, { recursive: true, force: true });
});

// Asynchronous, since the fake release server runs on this process's event loop.
function run(command, args, env) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { env: { ...process.env, ...env } });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("close", (status) => resolve({ status, stdout, stderr }));
  });
}

function freshDir(name) {
  return fs.mkdtempSync(path.join(root, `${name}-`));
}

const wrapperEnv = (cache) => ({ REVSCOPE_RELEASE_URL: baseUrl, REVSCOPE_CACHE_DIR: cache });

test("the wrapper's version is the CLI's", () => {
  const { version } = JSON.parse(fs.readFileSync(path.join(here, "../package.json"), "utf8"));
  const cargo = fs.readFileSync(path.join(here, "../../Cargo.toml"), "utf8");
  assert.equal(cargo.match(/^version = "(.+)"$/m)[1], version);
});

test("the wrapper fetches once, then passes arguments, stdout and exit status through", async () => {
  const cache = freshDir("cache");
  const first = await run(wrapper, ["whoami", "--json"], wrapperEnv(cache));
  assert.equal(first.status, 0, first.stderr);
  assert.equal(first.stdout, "ran: whoami --json\n", "progress must not reach stdout");

  requests.length = 0;
  const second = await run(wrapper, ["tables"], { ...wrapperEnv(cache), FAKE_EXIT: "7" });
  assert.equal(second.status, 7);
  assert.equal(second.stdout, "ran: tables\n");
  assert.deepEqual(requests, [], "the second run needs no download");
});

test("the wrapper refuses a download whose checksum doesn't match", async () => {
  const name = `/revscope-${target}.tar.gz.sha256`;
  const good = files[name];
  files[name] = Buffer.from(`${"0".repeat(64)}  x\n`);
  try {
    const cache = freshDir("cache");
    const result = await run(wrapper, ["tables"], wrapperEnv(cache));
    assert.equal(result.status, 1);
    assert.match(result.stderr, /checksum/);
    assert.equal(result.stdout, "");
  } finally {
    files[name] = good;
  }
});

test("the wrapper reports a release it can't find", async () => {
  const cache = freshDir("cache");
  const result = await run(wrapper, ["tables"], {
    ...wrapperEnv(cache),
    REVSCOPE_RELEASE_URL: `${baseUrl}/missing`,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /could not fetch/);
});

test("a network failure says to retry, as JSON when asked for", async () => {
  const cache = freshDir("cache");
  const result = await run(wrapper, ["--json", "whoami"], {
    ...wrapperEnv(cache),
    REVSCOPE_RELEASE_URL: "http://127.0.0.1:9",
  });
  assert.equal(result.status, 75);
  assert.equal(JSON.parse(result.stdout).error.exit_code, 75);
});

test("install.sh installs a working binary and says when it's not on PATH", async () => {
  const dir = freshDir("bin");
  const result = await run("sh", [installer], {
    REVSCOPE_RELEASE_URL: baseUrl,
    REVSCOPE_INSTALL_DIR: dir,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Installed revscope 9\.9\.9/);
  assert.match(result.stdout, /to your PATH/);
  const installed = await run(path.join(dir, "revscope"), ["tables"], {});
  assert.equal(installed.stdout, "ran: tables\n");
});

test("install.sh fails when the installed binary can't run", async () => {
  const stage = freshDir("stage");
  fs.writeFileSync(path.join(stage, "revscope"), "#!/no/such/interpreter\n", { mode: 0o755 });
  const archive = `revscope-${target}.tar.gz`;
  const tarball = path.join(root, "broken.tar.gz");
  spawnSync("tar", ["-czf", tarball, "-C", stage, "revscope"]);
  const bytes = fs.readFileSync(tarball);
  const good = [files[`/${archive}`], files[`/${archive}.sha256`]];
  files[`/${archive}`] = bytes;
  files[`/${archive}.sha256`] = Buffer.from(
    `${createHash("sha256").update(bytes).digest("hex")}  ${archive}\n`,
  );
  try {
    const result = await run("sh", [installer], {
      REVSCOPE_RELEASE_URL: baseUrl,
      REVSCOPE_INSTALL_DIR: freshDir("bin"),
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /doesn't run/);
  } finally {
    [files[`/${archive}`], files[`/${archive}.sha256`]] = good;
  }
});

test("install.sh refuses a download whose checksum doesn't match", async () => {
  const name = `/revscope-${target}.tar.gz.sha256`;
  const good = files[name];
  files[name] = Buffer.from(`${"0".repeat(64)}  x\n`);
  try {
    const dir = freshDir("bin");
    const result = await run("sh", [installer], {
      REVSCOPE_RELEASE_URL: baseUrl,
      REVSCOPE_INSTALL_DIR: dir,
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /checksum/);
    assert.equal(fs.existsSync(path.join(dir, "revscope")), false);
  } finally {
    files[name] = good;
  }
});
