#!/usr/bin/env bash
# Builds the image and runs it against test-server.mjs, a stand-in for RevScope's MCP endpoint on
# this machine, so it needs no network beyond the build. Linux only, for --network host.
# Usage: mcp-bridge/test.sh
set -euo pipefail
cd "$(dirname "$0")"

sh -n entrypoint.sh
for script in *.mjs; do node --check "$script"; done

# Unique per run, so worktrees checking at the same time don't test each other's image.
image=revscope-mcp-bridge-test-$$
volume=$image
port=$(node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port)); s.close(); })')
url=http://localhost:$port/v2/mcp
token=rsm_$(node -e 'process.stdout.write(require("node:crypto").randomBytes(16).toString("hex"))')
server=
cleanup() {
  [ -z "$server" ] || kill "$server" 2>/dev/null || true
  docker volume rm -f "$volume" >/dev/null 2>&1 || true
  docker image rm -f "$image" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker build -q -t "$image" . >/dev/null
docker volume create "$volume" >/dev/null
node test-server.mjs "$port" "$token" &
server=$!
sleep 1
kill -0 "$server"

# Runs the image and asserts its exit status and a line of its stderr.
expect() {
  local want_status=$1 want_stderr=$2 status=0 stderr
  shift 2
  stderr=$(docker run --rm -i --network host "$@" </dev/null 2>&1 >/dev/null) || status=$?
  if [ "$status" -ne "$want_status" ] || [[ $stderr != *"$want_stderr"* ]]; then
    echo "docker run $*: want status $want_status and '$want_stderr', got $status:" >&2
    echo "$stderr" >&2
    exit 1
  fi
}

# Runs the image as an MCP client would and asserts that test-server answered its initialize.
expect_connected() {
  node test-client.mjs docker run --rm -i --network host "$@" 2>/dev/null ||
    { echo "docker run $*: test-server didn't answer initialize" >&2; exit 1; }
}

# Saves a sign-in to $1 where mcp-remote 0.15 keeps it, with token $2, expiring in $3 milliseconds,
# or with the expires_at $4 when given.
save_sign_in() {
  local hash tokens
  hash=$(node -e 'process.stdout.write(require("node:crypto").createHash("md5").update(process.argv[1]).digest("hex"))' "$1")
  tokens=$(node -e 'process.stdout.write(JSON.stringify({access_token: process.argv[1], token_type: "Bearer", expires_at: process.argv[3] ?? Date.now() + Number(process.argv[2])}))' "$2" "$3" ${4+"$4"})
  docker run --rm -v "$volume:/data" -e TOKENS="$tokens" --entrypoint sh "$image" -c \
    "mkdir -p /data/mcp-remote-v1 && printf %s \"\$TOKENS\" > /data/mcp-remote-v1/${hash}_tokens.json"
}

expect 1 "not signed in to https://api.revscope.co/v2/mcp. Sign in once with:
  docker run -it --rm -p '127.0.0.1:3335:3335' -v revscope-mcp:/data revscope-mcp-bridge login" "$image"
query_url="$url?a=1&b='2'"
expect 1 "  docker run -it --rm -p '127.0.0.1:4000:4000' -v revscope-mcp:/data -e 'REVSCOPE_MCP_URL=$url?a=1&b='\''2'\''' -e 'REVSCOPE_MCP_CALLBACK_PORT=4000' revscope-mcp-bridge login" \
  -e REVSCOPE_MCP_URL="$query_url" -e REVSCOPE_MCP_CALLBACK_PORT=4000 "$image"
expect 2 "unknown command 'serve'" "$image" serve
expect 2 "REVSCOPE_MCP_TOKEN takes no arguments" -e REVSCOPE_MCP_TOKEN="$token" "$image" login

expect_connected -e REVSCOPE_MCP_URL="$url" -e REVSCOPE_MCP_TOKEN="$token" "$image"
# A token pasted with a line ending still reaches the server, as it does without the bridge.
expect_connected -e REVSCOPE_MCP_URL="$url" -e REVSCOPE_MCP_TOKEN="$token"$'\r\n' "$image"
# mcp-remote refuses plain http to anything but localhost unless the bridge passes --allow-http.
expect_connected -e REVSCOPE_MCP_URL="http://127.0.0.2:$port/v2/mcp" -e REVSCOPE_MCP_TOKEN="$token" "$image"
expect 1 "revscope-mcp-bridge: the token contains a line break." -e REVSCOPE_MCP_URL="$url" -e REVSCOPE_MCP_TOKEN="${token:0:20}"$'\n'"${token:20}" "$image"
expect 1 "rejected REVSCOPE_MCP_TOKEN" -e REVSCOPE_MCP_URL="$url" -e REVSCOPE_MCP_TOKEN="rsm_$(printf '0%.0s' {1..32})" "$image"
expect 1 "can't reach http://localhost:1/v2/mcp" -e REVSCOPE_MCP_URL=http://localhost:1/v2/mcp -e REVSCOPE_MCP_TOKEN="$token" "$image"

expect 1 "not signed in to $url" -e REVSCOPE_MCP_URL="$url" -v "$volume:/data" "$image"
save_sign_in "$url" "$token" 3600000
expect_connected -e REVSCOPE_MCP_URL="$url" -v "$volume:/data" "$image"
save_sign_in "$url" "$token" 30000
expect 1 "has expired" -e REVSCOPE_MCP_URL="$url" -v "$volume:/data" "$image"
save_sign_in "$url" "$token" 0 soon
expect 1 "isn't one mcp-remote can use" -e REVSCOPE_MCP_URL="$url" -v "$volume:/data" "$image"
save_sign_in "$url" "rsm_$(printf '1%.0s' {1..32})" 3600000
expect 1 "no longer accepts the saved sign-in" -e REVSCOPE_MCP_URL="$url" -v "$volume:/data" "$image"

# An MCP client stops the bridge with SIGTERM, which must not wait out Docker's grace period.
container=$(docker run -d -i --rm --network host -e REVSCOPE_MCP_URL="$url" -e REVSCOPE_MCP_TOKEN="$token" "$image")
sleep 2
started=$SECONDS
docker stop -t 10 "$container" >/dev/null
if [ $((SECONDS - started)) -ge 5 ]; then
  echo "the bridge took $((SECONDS - started))s to stop on SIGTERM" >&2
  exit 1
fi
echo "mcp-bridge tests passed"
