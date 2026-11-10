#!/bin/sh
# Speaks MCP over stdio to the client and Streamable HTTP to RevScope, through mcp-remote.
# See README.md for the three ways to run it.
set -eu

url=${REVSCOPE_MCP_URL:-https://api.revscope.co/v2/mcp}
# login and the proxy must use the same port: mcp-remote registers its OAuth client with a redirect
# URI on this port, and on another port it discards that registration, making the next login
# register again.
port=${REVSCOPE_MCP_CALLBACK_PORT:-3335}
bin=/bridge/node_modules/.bin
# mcp-remote refuses plain http anywhere but localhost, which inside the container is the container.
insecure=
case "$url" in http://*) insecure=--allow-http ;; esac

if [ -n "${REVSCOPE_MCP_TOKEN:-}" ]; then
	[ $# -eq 0 ] || { echo "revscope-mcp-bridge: REVSCOPE_MCP_TOKEN takes no arguments" >&2; exit 2; }
	node /bridge/preflight.mjs "$url" "$port"
	# The literal ${REVSCOPE_MCP_TOKEN} is substituted by mcp-remote, which keeps the token out of
	# the process list.
	# shellcheck disable=SC2016
	exec "$bin/mcp-remote" "$url" --transport http-only ${insecure:+"$insecure"} --header 'Authorization: Bearer ${REVSCOPE_MCP_TOKEN}'
fi

case "${1:-}" in
login)
	node /bridge/forward.mjs "$port" &
	"$bin/mcp-remote-client" "$url" "$port" --transport http-only ${insecure:+"$insecure"} --auth-timeout 600
	echo "revscope-mcp-bridge: signed in to $url. Your MCP client can start the bridge now." >&2
	;;
"")
	node /bridge/preflight.mjs "$url" "$port"
	exec "$bin/mcp-remote" "$url" "$port" --transport http-only ${insecure:+"$insecure"}
	;;
*)
	echo "revscope-mcp-bridge: unknown command '$1'; the only one is 'login'" >&2
	exit 2
	;;
esac
