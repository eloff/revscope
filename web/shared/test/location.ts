import { vi } from "vitest";

/**
 * jsdom doesn't implement navigation, so components that call
 * `window.location.assign` get a spy in its place. The rest of `location`
 * (pathname, search) keeps working off `pathname`.
 */
export function stubLocation(pathname = "/", search = "") {
  const assign = vi.fn();
  const url = new URL(`http://localhost:4322${pathname}${search}`);
  vi.stubGlobal("location", {
    assign,
    replace: vi.fn(),
    reload: vi.fn(),
    href: url.href,
    origin: url.origin,
    pathname: url.pathname,
    search: url.search,
    hash: "",
    host: url.host,
    hostname: url.hostname,
    protocol: url.protocol,
  });
  return assign;
}
