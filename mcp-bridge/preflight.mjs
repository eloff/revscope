// Checks the token mcp-remote is about to send before handing over to it. mcp-remote answers a
// missing or rejected token by starting an OAuth sign-in and waiting, with no timeout, for a
// browser nobody opens, so the client would hang instead of hearing why it can't connect.
// Usage: node preflight.mjs <MCP URL> <callback port>
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

const [url, port] = process.argv.slice(2);
const quote = (value) => `'${value.replaceAll("'", `'\\''`)}'`;
const login = [
  `docker run -it --rm -p ${quote(`127.0.0.1:${port}:${port}`)} -v revscope-mcp:/data`,
  process.env.REVSCOPE_MCP_URL && `-e ${quote(`REVSCOPE_MCP_URL=${url}`)}`,
  process.env.REVSCOPE_MCP_CALLBACK_PORT && `-e ${quote(`REVSCOPE_MCP_CALLBACK_PORT=${port}`)}`,
  "revscope-mcp-bridge login",
]
  .filter(Boolean)
  .join(" ");

const fromEnv = !!process.env.REVSCOPE_MCP_TOKEN;
const token = fromEnv ? process.env.REVSCOPE_MCP_TOKEN.trim() : await savedToken();
// fetch would refuse it with an error that quotes the whole header, token included.
if (/[\r\n]/.test(token)) fail("the token contains a line break.");

let response;
try {
  response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/json, text/event-stream",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: 0, method: "ping" }),
    // fetch drops Authorization on a cross-origin redirect, which would read as a rejected token.
    redirect: "manual",
    signal: AbortSignal.timeout(15000),
  });
} catch (error) {
  fail(`can't reach ${url}: ${error.cause?.message ?? error.message}`);
}
if (response.status >= 300 && response.status < 400) {
  const location = response.headers.get("location");
  fail(`${url} redirects to ${location}. Set REVSCOPE_MCP_URL to the MCP endpoint itself.`);
}
if (response.status === 401) {
  fail(
    fromEnv
      ? `${url} rejected REVSCOPE_MCP_TOKEN.`
      : `${url} no longer accepts the saved sign-in. Sign in again with:\n  ${login}`,
  );
}

// The token mcp-remote would send for this URL, failing where it would refuse it. Where it saves
// tokens is mcp-remote 0.15's: a hash of the URL alone, since the bridge passes no headers or OAuth
// options when signed in this way. test.sh checks mcp-remote still reads the file found here.
async function savedToken() {
  const hash = createHash("md5").update(url).digest("hex");
  const path = `${process.env.MCP_REMOTE_CONFIG_DIR}/mcp-remote-v1/${hash}_tokens.json`;
  let saved;
  try {
    saved = JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") fail(`not signed in to ${url}. Sign in once with:\n  ${login}`);
    fail(`can't read the saved sign-in at ${path}: ${error.message}`);
  }
  // The fields mcp-remote's schema checks; a file failing it reads to mcp-remote as no sign-in.
  const number = (value) => value === undefined || Number.isFinite(Number(value));
  const text = (value) => value === undefined || typeof value === "string";
  const usable =
    typeof saved?.access_token === "string" &&
    typeof saved.token_type === "string" &&
    number(saved.expires_in) &&
    number(saved.expires_at) &&
    ["scope", "refresh_token", "id_token", "requested_scope"].every((key) => text(saved[key]));
  if (!usable) {
    fail(
      `the saved sign-in at ${path} isn't one mcp-remote can use. Sign in again with:\n  ${login}`,
    );
  }
  // mcp-remote treats a token within a minute of expiry as expired, and RevScope issues no refresh
  // token, so it would start a sign-in.
  if (saved.expires_at && Date.now() >= saved.expires_at - 60_000) {
    fail(`the saved sign-in to ${url} has expired. Sign in again with:\n  ${login}`);
  }
  return saved.access_token;
}

function fail(message) {
  console.error(`revscope-mcp-bridge: ${message}`);
  process.exit(1);
}
