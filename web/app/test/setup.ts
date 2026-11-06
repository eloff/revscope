import "@testing-library/jest-dom/vitest";
import { cleanup } from "@solidjs/testing-library";
import { afterAll, afterEach, beforeAll } from "vitest";
import { forgetSignInProviders } from "../../shared/src/idTokenSignIn";
import { server } from "../../shared/test/server";

// jsdom lacks the layout APIs Ark's positioning and ApexCharts' wrappers reach for.
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= NoopResizeObserver;
// A file can opt into the node environment (astroConfig.test.ts), which has no DOM.
const hasDom = typeof window !== "undefined";
if (hasDom) {
  window.matchMedia ??= (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener() {},
      removeListener() {},
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
}

// An unmocked request never reaches a real server. MSW only rejects its fetch,
// which components catch and render as an error, so the test is failed here.
const unhandledRequests: string[] = [];
beforeAll(() => {
  server.listen({ onUnhandledRequest: "error" });
  server.events.on("request:unhandled", ({ request }) => {
    unhandledRequests.push(`${request.method} ${request.url}`);
  });
});
function failOnUnhandledRequests() {
  const unhandled = unhandledRequests.splice(0);
  if (unhandled.length > 0) {
    throw new Error(`Requests no handler answered:\n${unhandled.join("\n")}`);
  }
}
afterEach(() => {
  cleanup();
  server.resetHandlers();
  forgetSignInProviders();
  if (hasDom) localStorage.clear();
  failOnUnhandledRequests();
});
// A request a timer fires after the last test's body is only seen here, once
// pending work has had a moment to reach MSW.
afterAll(async () => {
  await new Promise((resolve) => setTimeout(resolve, 50));
  server.close();
  failOnUnhandledRequests();
});
