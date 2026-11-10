# RevScope MCP bridge

RevScope's MCP server is hosted at `https://api.revscope.co/v2/mcp`. Clients that support remote
MCP servers, such as Claude, Codex, Cursor and VS Code, should connect to that URL directly; see
[the AI docs](https://revscope.co/docs/ai/). This bridge is for clients that only start local
servers over stdio. It runs in Docker and relays to the hosted server with
[mcp-remote](https://github.com/geelen/mcp-remote), handling the OAuth sign-in for them.

It exposes the same read-only tools as the hosted server: `list_projects`, `get_schema` and
`run_query`.

## Build

```sh
docker build -t revscope-mcp-bridge https://github.com/eloff/revscope.git#main:mcp-bridge
```

## Sign in once

```sh
docker run -it --rm -p 127.0.0.1:3335:3335 -v revscope-mcp:/data revscope-mcp-bridge login
```

This prints a link to RevScope's consent page. Open it, approve, and your browser returns to
`localhost:3335`, which Docker forwards to the bridge. The access token is saved in the
`revscope-mcp` volume and lasts a year. To revoke it, use the danger zone in RevScope's settings,
then sign in again.

Port 3335 only needs to be free on your machine while you sign in. To use another one, pass
`-e REVSCOPE_MCP_CALLBACK_PORT=<port>` and publish that port instead. Pass the same variable when
running the bridge afterwards too: on a different port it discards the OAuth client it registered,
so the next sign-in registers a new one.

## Add it to your MCP client

```json
{
  "mcpServers": {
    "revscope": {
      "command": "docker",
      "args": ["run", "-i", "--rm", "-v", "revscope-mcp:/data", "revscope-mcp-bridge"]
    }
  }
}
```

No port is published here, so several clients can run the bridge at once. If the saved sign-in is
missing, expired or revoked when the bridge starts, it exits straight away and prints the command
to sign in again. A bridge that is already running when you revoke access waits for a sign-in it
can't complete, so its client's next request hangs; restart the client after signing in again.

## With a token instead

To skip the sign-in, pass an `rsm_` MCP token in `REVSCOPE_MCP_TOKEN`. MCP directories use this
with the server's demo token (`REVSCOPE_MCP_DEMO_TOKEN`), which reads only the public demo, to
check that the bridge starts and lists its tools.

```sh
docker run -i --rm -e REVSCOPE_MCP_TOKEN revscope-mcp-bridge
```

## A self-hosted instance

Point the bridge at your instance's MCP endpoint, in both the sign-in and the client config. Use
the endpoint itself rather than a URL that redirects to it. A plain `http://` URL works too, but
sends the token unencrypted, so keep it to a trusted network.

```sh
docker run -it --rm -p 127.0.0.1:3335:3335 -v revscope-mcp:/data \
  -e REVSCOPE_MCP_URL=https://revscope.example.com/v2/mcp revscope-mcp-bridge login
```

## Development

`./test.sh` builds the image and runs it against `test-server.mjs`, a stand-in for RevScope's MCP
endpoint, so it needs no account. It uses `--network host`, so it runs on Linux.
