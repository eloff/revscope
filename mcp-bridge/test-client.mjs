// Runs a command as an MCP client would and succeeds once it answers initialize as test-server,
// which it does only when the bridge sent the accepted token. Keeps stdin open until then, since
// mcp-remote shuts down when stdin ends.
// Usage: node test-client.mjs <command> [args...]
import { spawn } from "node:child_process";
import readline from "node:readline";

const [command, ...args] = process.argv.slice(2);
const bridge = spawn(command, args, { stdio: ["pipe", "pipe", "inherit"] });
const timer = setTimeout(() => finish(1, "no answer to initialize within 30s"), 30_000);

readline.createInterface({ input: bridge.stdout }).on("line", (line) => {
  if (line.includes('"name":"test-server"')) finish(0);
});
bridge.on("exit", (status) =>
  finish(1, `the bridge exited with status ${status} before answering`),
);
bridge.stdin.write(
  `${JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "test", version: "0" },
    },
  })}\n`,
);

function finish(status, message) {
  clearTimeout(timer);
  bridge.removeAllListeners("exit");
  if (message) console.error(message);
  bridge.stdin.end();
  bridge.kill();
  process.exit(status);
}
