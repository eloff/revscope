// A stand-in for RevScope's MCP endpoint, for test.sh. It accepts one token, answers initialize
// with its own server name, and rejects anything else with a 401 as RevScope does. It also listens
// on 127.0.0.2, a loopback address mcp-remote treats as remote, to test plain-http URLs.
// Usage: node test-server.mjs <port> <accepted token>
import http from "node:http";

const [port, accepted] = process.argv.slice(2);

function handle(request, response) {
  if (request.method !== "POST") return response.writeHead(405).end();
  if (request.headers.authorization !== `Bearer ${accepted}`) return response.writeHead(401).end();
  let body = "";
  request.on("data", (chunk) => {
    body += chunk;
  });
  request.on("end", () => {
    const message = JSON.parse(body);
    if (message.id === undefined) return response.writeHead(202).end();
    const result =
      message.method === "initialize"
        ? {
            protocolVersion: message.params.protocolVersion,
            capabilities: { tools: {} },
            serverInfo: { name: "test-server", version: "0" },
          }
        : {};
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ jsonrpc: "2.0", id: message.id, result }));
  });
}

for (const host of ["127.0.0.1", "127.0.0.2"]) {
  http
    .createServer(handle)
    .on("error", (error) => {
      console.error(`test-server on ${host}:${port}: ${error.message}`);
      process.exit(1);
    })
    .listen(Number(port), host);
}
