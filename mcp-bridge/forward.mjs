// mcp-remote 0.15 binds its OAuth callback to 127.0.0.1, which a port Docker publishes can't reach,
// since Docker forwards to the container's own address. This relays that address to the callback.
import net from "node:net";
import os from "node:os";

const port = Number(process.argv[2]);

for (const addresses of Object.values(os.networkInterfaces())) {
  for (const { address, family, internal } of addresses ?? []) {
    if (internal || family !== "IPv4") continue;
    net
      .createServer((browser) => {
        const callback = net.connect(port, "127.0.0.1");
        browser.pipe(callback).pipe(browser);
        browser.on("error", () => callback.destroy());
        callback.on("error", () => browser.destroy());
      })
      .on("error", (error) => console.error(`forwarding ${address}:${port}: ${error.message}`))
      .listen(port, address);
  }
}
