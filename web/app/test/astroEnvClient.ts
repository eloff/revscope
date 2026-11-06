// Stands in for Astro's `astro:env/client` virtual module, which only exists
// inside an Astro build. Requests to this host are answered by the MSW server
// in `test/server.ts`.
export const SERVER_HOST = "localhost:4320";
