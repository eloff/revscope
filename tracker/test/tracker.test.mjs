import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadTracker } from "./harness.mjs";

describe("tracker helpers", () => {
  let env;
  let rr;

  beforeEach(() => {
    env = loadTracker();
    rr = env.rr;
  });

  afterEach(() => {
    if (env && env.cleanup) {
      env.cleanup();
    }
  });

  function load(options) {
    const next = loadTracker(options);
    // Replace the suite-level env so afterEach cleans the latest load.
    if (env && env.cleanup && env !== next) {
      env.cleanup();
    }
    env = next;
    rr = next.rr;
    return next;
  }

  // Answer document.querySelectorAll from a selector -> elements map.
  function stubQuery(map) {
    env.window.document.querySelectorAll = (selector) => map[selector] || [];
  }

  describe("hasOwn / makeSet / parseList", () => {
    it("hasOwn detects own properties only", () => {
      const obj = Object.create({ inherited: 1 });
      obj.own = 2;
      expect(rr.hasOwn(obj, "own")).toBe(true);
      expect(rr.hasOwn(obj, "inherited")).toBe(false);
    });

    it("makeSet builds a truthy map and skips empty keys", () => {
      expect(rr.makeSet(["a", "", "b", null])).toEqual({
        a: true,
        b: true,
      });
    });

    it("parseList accepts arrays, csv strings, and empty values", () => {
      expect(rr.parseList(["a", "b"])).toEqual(["a", "b"]);
      expect(rr.parseList("a, b,c")).toEqual(["a", "b", "c"]);
      expect(rr.parseList("")).toEqual([]);
      expect(rr.parseList(undefined)).toEqual([]);
      expect(rr.parseList(null)).toEqual([]);
    });
  });

  describe("escapeRegExp", () => {
    it("escapes regex metacharacters", () => {
      expect(rr.escapeRegExp("a.b*c?")).toBe("a\\.b\\*c\\?");
      expect(new RegExp("^" + rr.escapeRegExp("foo[bar]") + "$").test("foo[bar]")).toBe(true);
    });
  });

  describe("isPlainObject / mergeObjects / isFinite", () => {
    it("isPlainObject accepts only Object instances", () => {
      expect(rr.isPlainObject({})).toBe(true);
      // Object.create(null) has no constructor === Object
      expect(rr.isPlainObject(Object.create(null))).toBeFalsy();
      expect(rr.isPlainObject([])).toBeFalsy();
      // Implementation short-circuits on falsy values (returns the value).
      expect(rr.isPlainObject(null)).toBeFalsy();
      expect(rr.isPlainObject("x")).toBeFalsy();
    });

    it("mergeObjects shallow-merges plain objects left to right", () => {
      expect(rr.mergeObjects({ a: 1 }, { b: 2 }, { a: 3 })).toEqual({
        a: 3,
        b: 2,
      });
      expect(rr.mergeObjects({ a: 1 }, null, [1], "x", { b: 2 })).toEqual({
        a: 1,
        b: 2,
      });
    });

    it("isFinite type checks", () => {
      expect(rr.isFinite(1.5)).toBe(true);
      expect(rr.isFinite(NaN)).toBe(false);
      expect(rr.isFinite("1")).toBe(false);
    });
  });

  describe("getDataAttribute", () => {
    it("reads data-* attributes and tolerates missing elements", () => {
      const el = env.createElement("div", { "data-mode": "hash" });
      expect(rr.getDataAttribute(el, "mode")).toBe("hash");
      expect(rr.getDataAttribute(null, "mode")).toBeFalsy();
    });
  });

  describe("createId", () => {
    it("createId returns 24 hex chars with timestamp prefix", () => {
      const id = rr.createId();
      expect(id).toMatch(/^[0-9a-f]{24}$/);
    });
  });

  describe("cookies", () => {
    it("getCookie / setCookie round-trip", () => {
      rr.setCookie("rr_test", "42", 3600);
      expect(rr.getCookie("rr_test")).toBe("42");
      expect(env.cookies.entries().rr_test).toBe("42");
    });

    it("getCookieDomain returns hostname for real domains and empty otherwise", () => {
      expect(rr.getCookieDomain()).toBe("example.com");

      // Hostname override is required so the tracker initializes on loopback;
      // cookie domain still follows location.hostname.
      const local = load({
        location: { hostname: "localhost", host: "localhost" },
        config: { hostname: "example.com" },
      });
      expect(local.rr.getCookieDomain()).toBe("");

      const ip = load({
        location: { hostname: "127.0.0.1", host: "127.0.0.1" },
        config: { hostname: "example.com" },
      });
      expect(ip.rr.getCookieDomain()).toBe("");
    });

    it("isReportableHostname rejects localhost and IPs", () => {
      expect(rr.isReportableHostname("example.com")).toBe(true);
      expect(rr.isReportableHostname("sub.example.com")).toBe(true);
      expect(rr.isReportableHostname("localhost")).toBe(false);
      expect(rr.isReportableHostname("")).toBe(false);
      expect(rr.isReportableHostname("127.0.0.1")).toBe(false);
      expect(rr.isReportableHostname("::1")).toBe(false);
      expect(rr.isReportableHostname("192.168.0.1")).toBe(false);
    });

    it("updateSessionId persists when consent allows", () => {
      rr.updateSessionId("00000000000003e7", true);
      expect(rr.getSessionId()).toBe("00000000000003e7");
      expect(env.cookies.entries().rsdemo).toBe("00000000000003e7");

      rr.updateSessionId("999", true);
      expect(rr.getSessionId()).toBe("00000000000003e7");
    });

    it("sends a session cookie as unchanged hex", () => {
      const withSession = load({
        cookies: { rsdemo: "184b6ca73b9aaff2" },
      });
      expect(withSession.requests[0].body).toContain("s=184b6ca73b9aaff2");
    });
  });

  describe("payloadQueryString / stringifyObj / metadata", () => {
    it("payloadQueryString encodes and drops nullish values", () => {
      const qs = rr.payloadQueryString({ a: "x y", b: null, c: undefined, d: 1 });
      expect(qs).toContain("a=x%20y");
      expect(qs).toContain("d=1");
      expect(qs).not.toContain("b=");
      expect(qs).not.toContain("c=");
    });

    it("stringifyObj rejects non-objects and oversized payloads", () => {
      expect(rr.stringifyObj({ a: 1 }, "metadata")).toBe('{"a":1}');
      expect(rr.stringifyObj(null, "metadata")).toBeUndefined();
      expect(rr.stringifyObj([], "metadata")).toBeUndefined();

      const big = { x: "y".repeat(5000) };
      expect(rr.stringifyObj(big, "metadata")).toBeUndefined();
    });

    it("serializeMetadataPayload serializes plain objects", () => {
      expect(rr.serializeMetadataPayload({ a: 1 })).toBe('{"a":1}');
      expect(rr.serializeMetadataPayload("nope")).toBeUndefined();
    });

    it("diffMetadata returns only new and changed key/value pairs", () => {
      expect(
        rr.diffMetadata(
          { unchanged: 1, changed: 2, added: true, nested: { same: true } },
          { unchanged: 1, changed: 1, nested: { same: true } },
        ),
      ).toEqual({ changed: 2, added: true });
      expect(rr.serializeMetadataDiff({ unchanged: 1 }, { unchanged: 1 })).toBeUndefined();
    });

    it("collectMetadata supports object and function config", () => {
      const withObject = load({
        config: {
          projectName: "demo",
          metadata: { plan: "pro" },
        },
      });
      expect(withObject.rr.collectMetadata({ type: "pageview" })).toEqual({
        plan: "pro",
      });

      const withFn = load({
        config: {
          projectName: "demo",
          metadata: (ctx) => ({ t: ctx.type }),
        },
      });
      expect(withFn.rr.collectMetadata({ type: "event" })).toEqual({ t: "event" });

      const throwing = load({
        config: {
          projectName: "demo",
          metadata: () => {
            throw new Error("boom");
          },
        },
        warn: vi.fn(),
      });
      expect(throwing.rr.collectMetadata({})).toBeUndefined();
      expect(throwing.window.console.warn).toHaveBeenCalled();
    });
  });

  describe("extractQueryParams", () => {
    it("keeps utm_/rr_/allowlisted/click-id params and strips prefixes", () => {
      const withQuery = load({
        location: {
          search: "?utm_source=google&rr_campaign=spring&gclid=abc&noise=1&ref=home",
        },
      });
      expect(withQuery.rr.extractQueryParams()).toEqual({
        source: "google",
        campaign: "spring",
        gclid: "abc",
        ref: "home",
      });
    });

    it("percent-decodes keys and values", () => {
      const encoded = load({
        location: {
          search: "?ref=https%3A%2F%2Fwww.google.com%2F&utm_term=hello%20world&utm_content=a%2Bb",
        },
      });
      expect(encoded.rr.extractQueryParams()).toEqual({
        ref: "https://www.google.com/",
        term: "hello world",
        content: "a+b",
      });
    });

    it("keeps malformed percent sequences as raw text", () => {
      const malformed = load({
        location: {
          search: "?ref=bad%zz&utm_source=ok",
        },
      });
      expect(malformed.rr.extractQueryParams()).toEqual({
        ref: "bad%zz",
        source: "ok",
      });
    });

    it("returns undefined when search is empty or only noise", () => {
      expect(rr.extractQueryParams()).toBeUndefined();

      const noise = load({
        location: { search: "?foo=1&bar=2" },
      });
      expect(noise.rr.extractQueryParams()).toBeUndefined();
    });
  });

  describe("parseDeclarativeValue", () => {
    it("parses event names and key=value metadata", () => {
      expect(rr.parseDeclarativeValue("buy")).toEqual({
        name: "buy",
        metadata: undefined,
      });
      expect(rr.parseDeclarativeValue("start-trial plan=growth billing=yearly")).toEqual({
        name: "start-trial",
        metadata: { plan: "growth", billing: "yearly" },
      });
      expect(rr.parseDeclarativeValue("checkout plan='pro annual' note=\"say hi\"")).toEqual({
        name: "checkout",
        metadata: { plan: "pro annual", note: "say hi" },
      });
      expect(rr.parseDeclarativeValue("plan=growth")).toBeUndefined();
      expect(rr.parseDeclarativeValue("")).toBeUndefined();
      expect(rr.parseDeclarativeValue(null)).toBeUndefined();
    });
  });

  describe("parseDeclarativeMetadata / applyDeclarativeMetadata", () => {
    it("parses space-separated key=value pairs", () => {
      expect(rr.parseDeclarativeMetadata("plan=growth billing=yearly")).toEqual({
        plan: "growth",
        billing: "yearly",
      });
      expect(rr.parseDeclarativeMetadata("plan='pro annual' note=\"say hi\"")).toEqual({
        plan: "pro annual",
        note: "say hi",
      });
      expect(rr.parseDeclarativeMetadata("  plan=growth  ")).toEqual({ plan: "growth" });
      expect(rr.parseDeclarativeMetadata("buy")).toBeUndefined();
      expect(rr.parseDeclarativeMetadata("")).toBeUndefined();
      expect(rr.parseDeclarativeMetadata(null)).toBeUndefined();
    });

    it("handles empty, unterminated, and malformed values", () => {
      expect(rr.parseDeclarativeMetadata("plan= note=''")).toEqual({ plan: "", note: "" });
      expect(rr.parseDeclarativeMetadata("note='pro annual")).toEqual({ note: "pro annual" });
      expect(rr.parseDeclarativeMetadata("plan=a=b")).toEqual({ plan: "a=b" });
      expect(rr.parseDeclarativeMetadata("note=\"say 'hi'\"")).toEqual({ note: "say 'hi'" });
      // A pair that is not key=value stops parsing at that point.
      expect(rr.parseDeclarativeMetadata("plan=growth oops region=us")).toEqual({
        plan: "growth",
      });
    });

    it("scans data-revscope-metadata and applies like the metadata command", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
        config: {
          projectName: "demo",
          metadata: { plan: "free", stable: "same" },
        },
      });
      rr.flushTrackingQueue(false);

      const first = env.createElement("div", {
        "data-revscope-metadata": "plan=pro region=us",
      });
      const second = env.createElement("div", {
        "data-revscope-metadata": "region=eu experiment=pricing-v2",
      });
      env.window.document.querySelectorAll = (selector) => {
        if (selector === "[data-revscope-metadata]") {
          return [first, second];
        }
        return [];
      };

      rr.applyDeclarativeMetadata();

      expect(rr.getPageMetadata()).toEqual({
        plan: "pro",
        stable: "same",
        region: "eu",
        experiment: "pricing-v2",
      });
      const updates = rr.getBatchQueue().filter((payload) => payload.ty === "metadata");
      expect(updates).toHaveLength(1);
      expect(JSON.parse(updates[0].meta)).toEqual({
        plan: "pro",
        region: "eu",
        experiment: "pricing-v2",
      });
    });

    it("a rescan keeps metadata the site set through the metadata command", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
        config: { projectName: "demo" },
      });
      const marked = env.createElement("div", {
        "data-revscope-metadata": "plan=pro",
      });
      stubQuery({ "[data-revscope-metadata]": [marked] });

      rr.applyDeclarativeMetadata();
      expect(rr.getPageMetadata().plan).toBe("pro");

      rr.updateMetadata({ plan: "enterprise" });
      rr.flushTrackingQueue(false);
      rr.applyDeclarativeMetadata();

      expect(rr.getPageMetadata().plan).toBe("enterprise");
      expect(rr.getBatchQueue()).toEqual([]);
    });

    it("a rescan applies a declarative value that changed in the DOM", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
        config: { projectName: "demo" },
      });
      const marked = env.createElement("div", {
        "data-revscope-metadata": "plan=pro",
      });
      stubQuery({ "[data-revscope-metadata]": [marked] });

      rr.applyDeclarativeMetadata();
      rr.flushTrackingQueue(false);
      marked.setAttribute("data-revscope-metadata", "plan=growth");
      rr.applyDeclarativeMetadata();

      expect(rr.getPageMetadata().plan).toBe("growth");
      const updates = rr.getBatchQueue().filter((payload) => payload.ty === "metadata");
      expect(updates).toHaveLength(1);
      expect(JSON.parse(updates[0].meta)).toEqual({ plan: "growth" });
    });
  });

  describe("rescanning dynamic content", () => {
    const visibleSelector = "[data-revscope-visible]";

    function trackedEvents() {
      return rr
        .getBatchQueue()
        .filter((payload) => payload.ty === "event")
        .map((payload) => payload.ev);
    }

    function elementRecord(node) {
      return { type: "childList", addedNodes: [node] };
    }

    afterEach(() => {
      vi.useRealTimers();
    });

    it("mutationsNeedScan only accepts element and attribute changes", () => {
      expect(rr.mutationsNeedScan([{ type: "attributes" }])).toBe(true);
      expect(rr.mutationsNeedScan([elementRecord({ nodeType: 1 })])).toBe(true);
      expect(rr.mutationsNeedScan([])).toBe(false);
      expect(rr.mutationsNeedScan([elementRecord({ nodeType: 3 })])).toBe(false);
      expect(
        rr.mutationsNeedScan([
          { type: "childList", addedNodes: [{ nodeType: 3 }, { nodeType: 1 }] },
        ]),
      ).toBe(true);
    });

    it("observes the whole document for declarative attributes", () => {
      expect(env.observers).toHaveLength(1);
      expect(env.observers[0].targets).toHaveLength(1);
      const { target, options } = env.observers[0].targets[0];
      expect(target).toBe(env.window.document);
      expect(options.childList).toBe(true);
      expect(options.subtree).toBe(true);
      expect(options.attributeFilter).toEqual(["data-revscope-visible", "data-revscope-metadata"]);
    });

    it("fires visible events for elements added after the first scan", () => {
      vi.useFakeTimers();
      const banner = env.createElement("div", {
        "data-revscope-visible": "hero-seen plan=pro",
      });
      stubQuery({ [visibleSelector]: [banner] });

      env.observers[0].emit([elementRecord(banner)]);
      expect(trackedEvents()).toEqual([]);

      vi.advanceTimersByTime(250);
      expect(trackedEvents()).toEqual(["hero-seen"]);
      const event = rr.getBatchQueue().find((payload) => payload.ev === "hero-seen");
      expect(JSON.parse(event.meta)).toEqual({ plan: "pro" });
    });

    it("coalesces a burst of mutations into a single scan", () => {
      vi.useFakeTimers();
      let scans = 0;
      env.window.document.querySelectorAll = (selector) => {
        if (selector === visibleSelector) {
          scans += 1;
        }
        return [];
      };

      const node = { nodeType: 1 };
      env.observers[0].emit([elementRecord(node)]);
      env.observers[0].emit([elementRecord(node)]);
      env.observers[0].emit([elementRecord(node)]);
      vi.advanceTimersByTime(250);
      expect(scans).toBe(1);

      // The window closes after the scan, so later mutations scan again.
      env.observers[0].emit([elementRecord(node)]);
      vi.advanceTimersByTime(250);
      expect(scans).toBe(2);
    });

    it("skips scanning when only text nodes changed", () => {
      vi.useFakeTimers();
      const banner = env.createElement("div", {
        "data-revscope-visible": "hero-seen",
      });
      stubQuery({ [visibleSelector]: [banner] });

      env.observers[0].emit([elementRecord({ nodeType: 3 })]);
      vi.advanceTimersByTime(250);
      expect(trackedEvents()).toEqual([]);
    });

    it("polls on an interval when MutationObserver is unavailable", () => {
      vi.useFakeTimers();
      load({ mutationObserver: false });
      expect(env.observers).toHaveLength(0);

      const first = env.createElement("div", {
        "data-revscope-visible": "hero-seen",
      });
      stubQuery({ [visibleSelector]: [first] });
      vi.advanceTimersByTime(2000);
      expect(trackedEvents()).toEqual(["hero-seen"]);

      const second = env.createElement("div", {
        "data-revscope-visible": "footer-seen",
      });
      stubQuery({ [visibleSelector]: [second] });
      vi.advanceTimersByTime(2000);
      expect(trackedEvents()).toEqual(["hero-seen", "footer-seen"]);
    });
  });

  describe("looksLikeSameTabNavigation", () => {
    function clickEvent(overrides = {}) {
      return {
        defaultPrevented: false,
        metaKey: false,
        ctrlKey: false,
        shiftKey: false,
        altKey: false,
        button: 0,
        ...overrides,
      };
    }

    it("detects same-tab http navigations", () => {
      const anchor = env.createElement("a", { href: "/signup" });
      expect(rr.looksLikeSameTabNavigation(clickEvent(), anchor)).toBe(true);
    });

    it("rejects prevented defaults, modifiers, fragments, and new tabs", () => {
      const anchor = env.createElement("a", { href: "/signup" });
      expect(rr.looksLikeSameTabNavigation(clickEvent({ defaultPrevented: true }), anchor)).toBe(
        false,
      );
      expect(rr.looksLikeSameTabNavigation(clickEvent({ metaKey: true }), anchor)).toBe(false);
      expect(
        rr.looksLikeSameTabNavigation(clickEvent(), env.createElement("a", { href: "#pricing" })),
      ).toBe(false);
      expect(
        rr.looksLikeSameTabNavigation(
          clickEvent(),
          env.createElement("a", { href: "/signup", target: "_blank" }),
        ),
      ).toBe(false);
      expect(rr.looksLikeSameTabNavigation(clickEvent(), env.createElement("button"))).toBe(false);
    });
  });

  describe("outboundDestination", () => {
    it("returns external http(s) hostnames", () => {
      const anchor = env.createElement("a", {
        href: "https://other.com/path",
      });
      anchor.protocol = "https:";
      anchor.hostname = "other.com";
      expect(rr.outboundDestination(anchor)).toBe("other.com");
    });

    it("ignores same-host links", () => {
      const anchor = env.createElement("a", {
        href: "https://example.com/path",
      });
      anchor.protocol = "https:";
      anchor.hostname = "example.com";
      expect(rr.outboundDestination(anchor)).toBeUndefined();
    });

    it("returns mailto: plus email domain", () => {
      const anchor = env.createElement("a", {
        href: "mailto:Sales@Acme.COM?subject=hi",
      });
      anchor.protocol = "mailto:";
      expect(rr.outboundDestination(anchor)).toBe("mailto:acme.com");
    });
  });

  describe("getTrackedPath", () => {
    it("returns pathname and applies pathRewriter", () => {
      expect(rr.getTrackedPath()).toBe("/");
      expect(rr.getTrackedPath("/custom")).toBe("/custom");

      const rewritten = load({
        config: {
          projectName: "demo",
          pathRewriter: (ctx) => "/rewritten" + ctx.path,
        },
      });
      expect(rewritten.rr.getTrackedPath("/x")).toBe("/rewritten/x");
    });

    it("ignores configured pages and supports wildcards", () => {
      const ignored = load({
        config: {
          projectName: "demo",
          ignorePages: ["/secret", "/admin/*"],
        },
        warn: vi.fn(),
      });
      expect(ignored.rr.getTrackedPath("/secret")).toBeUndefined();
      expect(ignored.rr.getTrackedPath("/admin/users")).toBeUndefined();
      expect(ignored.rr.getTrackedPath("/ok")).toBe("/ok");
    });

    it("appends hash in hash tracking mode", () => {
      const hashed = load({
        config: {
          projectName: "demo",
          mode: "hash",
        },
        location: {
          pathname: "/",
          hash: "#/dashboard?x=1",
        },
      });
      expect(hashed.rr.getTrackedPath()).toBe("/#/dashboard");
    });
  });

  describe("idle tracking", () => {
    it("accumulates idle time after the threshold", () => {
      const start = 1_000_000;
      const threshold = rr.getIdleThresholdMs();
      rr.setPageEngagementStartedAt(start);
      rr.setLastScrollActivityAt(start);
      rr.setIdleStartedAt(undefined);
      rr.setIdleDuration(0);

      const beforeThreshold = start + threshold - 1;
      expect(rr.calculateIdleDuration(beforeThreshold)).toBe(0);

      // Idle period starts at lastScrollActivityAt once the threshold is crossed.
      const afterThreshold = start + threshold + 5_000;
      expect(rr.calculateIdleDuration(afterThreshold)).toBe(threshold + 5_000);

      rr.setIdleStartedAt(start);
      rr.setLastScrollActivityAt(start);
      rr.setIdleDuration(0);
      rr.finishIdlePeriod(start + 10_000);
      expect(rr.getIdleDuration()).toBe(10_000);
      expect(rr.getIdleStartedAt()).toBeUndefined();
    });

    it("updateIdleState starts an idle period after the threshold", () => {
      const start = 2_000_000;
      const threshold = rr.getIdleThresholdMs();
      rr.setLastScrollActivityAt(start);
      rr.setIdleStartedAt(undefined);

      rr.updateIdleState(start + threshold - 1);
      expect(rr.getIdleStartedAt()).toBeUndefined();

      rr.updateIdleState(start + threshold);
      expect(rr.getIdleStartedAt()).toBe(start);
    });

    it("recordScrollActivity finishes idle and resets the activity clock", () => {
      const start = 3_000_000;
      const nowSpy = vi.spyOn(Date, "now").mockReturnValue(start + 20_000);
      rr.setPageEngagementStartedAt(start);
      rr.setLastScrollActivityAt(start);
      rr.setIdleStartedAt(start);
      rr.setIdleDuration(0);

      rr.recordScrollActivity();

      expect(rr.getIdleDuration()).toBe(20_000);
      expect(rr.getIdleStartedAt()).toBeUndefined();
      nowSpy.mockRestore();
    });
  });

  describe("visibility helpers", () => {
    it("isElementHidden detects hidden attribute and CSS", () => {
      const el = env.createElement("div");
      expect(rr.isElementHidden(el)).toBe(false);

      el.setAttribute("hidden", "");
      expect(rr.isElementHidden(el)).toBe(true);
      el.removeAttribute("hidden");

      el.style.display = "none";
      expect(rr.isElementHidden(el)).toBe(true);
      el.style.display = "";
      el.style.visibility = "hidden";
      expect(rr.isElementHidden(el)).toBe(true);
    });

    it("isElementVisible requires viewport intersection", () => {
      const el = env.createElement("div");
      el._rect = {
        top: 10,
        left: 10,
        bottom: 110,
        right: 110,
        width: 100,
        height: 100,
      };
      expect(rr.isElementVisible(el)).toBe(true);

      el._rect = {
        top: 2000,
        left: 0,
        bottom: 2100,
        right: 100,
        width: 100,
        height: 100,
      };
      expect(rr.isElementVisible(el)).toBe(false);

      // Zero-area named-anchor style target intersecting viewport
      el._rect = {
        top: 10,
        left: 10,
        bottom: 10,
        right: 10,
        width: 0,
        height: 0,
      };
      expect(rr.isElementVisible(el)).toBe(true);
    });
  });

  describe("clickCooldownElapsed", () => {
    it("throttles repeated fires within the cooldown window", () => {
      expect(rr.clickCooldownElapsed("buy")).toBe(true);
      expect(rr.clickCooldownElapsed("buy")).toBe(false);
      expect(rr.clickCooldownElapsed("other")).toBe(true);
    });
  });

  describe("applyIngestResponse", () => {
    it("updates session id from the ingest response", () => {
      rr.applyIngestResponse(
        JSON.stringify({
          visitor_timestamp_id: "000000000000022b",
          storage_allowed: true,
        }),
      );
      expect(rr.getSessionId()).toBe("000000000000022b");
      expect(env.cookies.entries().rsdemo).toBe("000000000000022b");

      const payload = rr.payloadWithRuntimeState({ ty: "pageview", pg: "abc" });
      expect(payload.spg).toBeUndefined();
      expect(payload.s).toBe("000000000000022b");
    });

    it("ignores invalid JSON", () => {
      const warn = vi.fn();
      const local = load({ warn });
      local.rr.applyIngestResponse("{not-json");
      expect(warn).toHaveBeenCalled();
    });

    it("refreshes the session cookie when storage is still allowed", () => {
      load({
        cookies: { rsdemo: "000000000000022b" },
      });
      rr.applyIngestResponse(
        JSON.stringify({
          visitor_timestamp_id: "000000000000022b",
          storage_allowed: true,
        }),
      );
      expect(env.cookies.entries().rsdemo).toBe("000000000000022b");
    });
  });

  describe("batch helpers", () => {
    it("never sends the user agent, the server reads it from the request header", () => {
      load({ navigator: { userAgent: "Mozilla/5.0 Mixed CASE" } });
      expect(rr.getBasePayload().ua).toBeUndefined();
      expect(rr.payloadWithRuntimeState({ ty: "event" }).ua).toBeUndefined();
      expect(rr.batchItemPayload({ ty: "event" }).ua).toBeUndefined();
      expect(rr.batchEnvelopePayload().ua).toBeUndefined();
    });

    it("batchItemPayload strips envelope fields", () => {
      const item = rr.batchItemPayload({
        batch: 1,
        v: "1",
        s: "sess",
        c: true,
        h: "host",
        st: 123,
        ty: "event",
        ev: "click",
      });
      expect(item.batch).toBeUndefined();
      expect(item.v).toBeUndefined();
      expect(item.s).toBeUndefined();
      expect(item.c).toBeUndefined();
      expect(item.h).toBeUndefined();
      expect(item.st).toBeUndefined();
      expect(item.ty).toBe("event");
      expect(item.ev).toBe("click");
      expect(typeof item.t).toBe("number");
    });

    it("keeps the item language, which only the context hit carries", () => {
      expect(rr.batchItemPayload({ ty: "context", l: "en-US" }).l).toBe("en-US");
      expect(rr.batchEnvelopePayload().l).toBeUndefined();
    });

    it("batchEnvelopePayload includes hostname and send time", () => {
      const envelope = rr.batchEnvelopePayload();
      expect(envelope.batch).toBe(1);
      expect(envelope.pid).toBeUndefined();
      expect(envelope.h).toBe("example.com");
      expect(envelope.v).toBe(rr.version);
      expect(typeof envelope.st).toBe("number");
      expect(rr.batchItemPayload({ ty: "event" }).st).toBeUndefined();
    });

    it("flushTrackingQueue sends the envelope send time alongside item times", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      rr.flushTrackingQueue(false);
      rr.enqueueTrackingRequest({ ty: "event", ev: "aged" });
      const itemTime = rr.getBatchQueue()[0].t;

      rr.flushTrackingQueue(false);

      const body = new URLSearchParams(env.requests[env.requests.length - 1].body);
      expect(Number(body.get("st"))).toBeGreaterThanOrEqual(itemTime);
      expect(JSON.parse(body.get("items"))[0].t).toBe(itemTime);
    });

    it("enqueueTrackingRequest buffers when session cookies exist", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      // Initial pageview stays buffered when cookies already exist.
      rr.flushTrackingQueue(false);
      expect(rr.getBatchQueue()).toHaveLength(0);

      rr.enqueueTrackingRequest({ ty: "event", ev: "buffered" });

      expect(rr.getBatchQueue()).toHaveLength(1);
      expect(rr.getBatchQueue()[0].ev).toBe("buffered");
      expect(typeof rr.getBatchQueue()[0].t).toBe("number");
    });

    it("flushTrackingQueue sends buffered items", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      rr.flushTrackingQueue(false);
      rr.enqueueTrackingRequest({ ty: "event", ev: "flush-me" });
      expect(rr.getBatchQueue()).toHaveLength(1);

      const requestsBefore = env.requests.length;
      expect(rr.flushTrackingQueue(false)).toBe(true);
      expect(rr.getBatchQueue()).toHaveLength(0);
      expect(env.requests.length).toBe(requestsBefore + 1);
      expect(env.requests[env.requests.length - 1].body).toContain("flush-me");
    });

    it("enqueueTrackingRequest with a callback flushes immediately", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      rr.flushTrackingQueue(false);
      const onComplete = vi.fn();
      const requestsBefore = env.requests.length;

      rr.enqueueTrackingRequest({ ty: "event", ev: "callback-flush" }, onComplete);

      expect(rr.getBatchQueue()).toHaveLength(0);
      expect(env.requests.length).toBe(requestsBefore + 1);
      expect(onComplete).toHaveBeenCalledOnce();
    });

    it("flushTrackingQueue is a no-op when the queue is empty", () => {
      // The default ingest response allows storage, which queues a context hit.
      rr.flushTrackingQueue(false);
      expect(rr.getBatchQueue()).toHaveLength(0);
      expect(rr.flushTrackingQueue(false)).toBe(false);
    });
  });

  describe("query parameter allowlists", () => {
    it("getAllowedParameterMap includes defaults and includeParams", () => {
      load({
        config: {
          projectName: "demo",
          includeParams: "custom_id, promo",
        },
      });
      const map = rr.getAllowedParameterMap();
      expect(map.ref).toBe(true);
      expect(map.source).toBe(true);
      expect(map.custom_id).toBe(true);
      expect(map.promo).toBe(true);
      expect(map.noise).toBeUndefined();
    });

    it("getClickIdParameterPattern matches known click-id keys", () => {
      const pattern = rr.getClickIdParameterPattern();
      expect(pattern.test("gclid")).toBe(true);
      expect(pattern.test("FBCLID")).toBe(true);
      expect(pattern.test("msclkid")).toBe(true);
      expect(pattern.test("utm_source")).toBe(false);
      expect(pattern.test("noise")).toBe(false);
    });
  });

  describe("scroll depth accessors", () => {
    it("setMaxScrollDepth / getMaxScrollDepth round-trip", () => {
      rr.setMaxScrollDepth(42);
      expect(rr.getMaxScrollDepth()).toBe(42);
    });
  });

  describe("getSession / consentStatus / updateMetadata", () => {
    it("getSession invokes the callback with session or page id", () => {
      const callback = vi.fn();
      rr.getSession(callback);
      expect(callback).toHaveBeenCalledWith(rr.getSessionId() || rr.getPageId());

      const warn = vi.fn();
      load({ warn });
      rr.getSession("not-a-function");
      expect(warn).toHaveBeenCalledWith(
        "revscope:",
        "callback must be a function, not: not-a-function",
      );
    });

    it("consentStatus invokes the callback once the requirement is known", () => {
      const callback = vi.fn();
      rr.consentStatus(callback);
      expect(callback).toHaveBeenCalledWith("not_required");

      const warn = vi.fn();
      load({ warn });
      rr.consentStatus(null);
      expect(warn).toHaveBeenCalledWith("revscope:", "callback must be a function, not: null");
    });

    it("updateMetadata merges page metadata and tracks lastSentMetadata", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
        config: {
          projectName: "demo",
          metadata: { plan: "free", stable: "same" },
        },
      });
      // Send the initial pageview metadata so lastSentMetadata is seeded.
      rr.flushTrackingQueue(false);

      rr.updateMetadata({ plan: "pro", region: "us" });
      expect(rr.getPageMetadata()).toEqual({
        plan: "pro",
        stable: "same",
        region: "us",
      });
      expect(rr.getLastSentMetadata()).toEqual({
        plan: "pro",
        stable: "same",
        region: "us",
      });

      rr.updateMetadata({ plan: "pro", region: "eu" });
      expect(rr.getLastSentMetadata()).toEqual({
        plan: "pro",
        stable: "same",
        region: "eu",
      });

      const updates = rr.getBatchQueue().filter((payload) => payload.ty === "metadata");
      expect(updates).toHaveLength(2);
      expect(JSON.parse(updates[1].meta)).toEqual({ region: "eu" });
    });

    it("updateMetadata rejects non-objects and disabled tracking", () => {
      const warn = vi.fn();
      load({
        warn,
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      rr.updateMetadata("nope");
      expect(warn).toHaveBeenCalledWith("revscope:", "metadata must be an object");

      rr.setConsent("disabled");
      rr.updateMetadata({ plan: "pro" });
      expect(rr.getPageMetadata()).toEqual({});
      expect(rr.getBatchQueue()).toHaveLength(0);
    });
  });

  describe("dispatchCommand", () => {
    it("rejects invalid commands and unknown actions", () => {
      const warn = vi.fn();
      load({ warn });

      rr.dispatchCommand(null);
      rr.dispatchCommand([]);
      rr.dispatchCommand(["config", {}]);
      rr.dispatchCommand(["not_an_action"]);

      const messages = warn.mock.calls.map((call) => String(call[1]));
      expect(messages).toContain("command must be a non-empty array");
      expect(messages).toContain("unknown action: config");
      expect(messages).toContain("unknown action: not_an_action");
    });

    it("routes known actions", () => {
      load({
        cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
      });
      rr.dispatchCommand(["event", "via-command"]);
      expect(rr.getBatchQueue().some((payload) => payload.ev === "via-command")).toBe(true);
    });
  });
});

describe("tracker public API", () => {
  const envs = [];

  afterEach(() => {
    while (envs.length) {
      const env = envs.pop();
      if (env.cleanup) {
        env.cleanup();
      }
    }
  });

  function load(options) {
    const env = loadTracker(options);
    envs.push(env);
    return env;
  }

  it("sends a crawler hit instead of a pageview when webdriver is set", () => {
    const env = load({ navigator: { webdriver: true } });
    expect(env.requests).toHaveLength(1);
    expect(env.requests[0].body).toContain("ty=crawler");
    expect(env.requests[0].body).toContain("h=example.com");
    expect(env.requests[0].body).toContain("p=%2F");
    expect(env.requests[0].body).not.toContain("ty=pageview");
  });

  it("sends a crawler hit for a PhantomJS global", () => {
    const env = load({
      beforeLoad(windowObj) {
        windowObj._phantom = {};
      },
    });
    expect(env.requests[0].body).toContain("ty=crawler");
  });

  it("tracks a pageview for a browser with the Phantom wallet's global", () => {
    const env = load({
      beforeLoad(windowObj) {
        windowObj.phantom = { solana: {} };
      },
    });
    expect(env.requests[0].body).toContain("ty=pageview");
    expect(env.requests[0].body).not.toContain("ty=crawler");
  });

  it("sends a crawler hit when crypto.getRandomValues repeats", () => {
    const env = load({
      beforeLoad(windowObj) {
        windowObj.crypto.getRandomValues = (arr) => {
          for (let i = 0; i < arr.length; i++) {
            arr[i] = 7;
          }
          return arr;
        };
      },
    });
    expect(env.requests).toHaveLength(1);
    expect(env.requests[0].body).toContain("ty=crawler");
    expect(env.requests[0].body).not.toContain("ty=pageview");
  });

  it("tracks a pageview when crypto.getRandomValues is missing", () => {
    const env = load({
      beforeLoad(windowObj) {
        windowObj.crypto = {};
      },
    });
    expect(env.requests[0].body).toContain("ty=pageview");
    expect(env.requests[0].body).not.toContain("ty=crawler");
  });

  it("loads and tracks an initial pageview", () => {
    const env = load();
    const dispatch = env.window.dispatch;
    expect(typeof dispatch).toBe("function");
    expect(dispatch.q).toEqual([]);
    expect(dispatch.projectName).toBe("demo");
    expect(Object.hasOwn(env.window, "revscope")).toBe(false);
    expect(env.rr.getPageId()).toMatch(/^[0-9a-f]{24}$/);
    expect(env.requests.length).toBeGreaterThanOrEqual(1);
    expect(env.requests[0].method).toBe("POST");
    expect(env.requests[0].url).toMatch(/\/demo$/);
    expect(env.requests[0].body).toContain("ty=pageview");
    expect(env.requests[0].body).not.toContain("pid=");
  });

  it.each(["", "ab", "bad-name", "bad name", "Demo", "REPLACE_WITH_PROJECT_NAME"])(
    "rejects invalid project name stub %j",
    (projectName) => {
      const warn = vi.fn();
      expect(() => load({ projectName, warn })).toThrow(/did not export __rrTest/);
      expect(warn).toHaveBeenCalledWith(
        "revscope:",
        "Project name missing; install the stub script",
      );
    },
  );

  it.each(["my_project", "123demo", "a1_b2", "abc"])("allows project name %j", (projectName) => {
    const env = load({ projectName });
    expect(env.rr.getProjectName()).toBe(projectName);
    expect(env.window.dispatch.projectName).toBe(projectName);
    expect(env.requests[0].url).toContain("/" + projectName);
    expect(env.requests[0].body).not.toContain("pid=");
  });

  it("disables tracking when the stub is missing", () => {
    const warn = vi.fn();
    expect(() => load({ installStub: false, warn })).toThrow(/did not export __rrTest/);
    expect(warn).toHaveBeenCalledWith("revscope:", "Project name missing; install the stub script");
  });

  it("rejects multiple queue stub handoffs", () => {
    const warn = vi.fn();
    expect(() =>
      load({
        warn,
        extraStubs: [{ projectName: "other", config: { name: "otherDispatch" } }],
      }),
    ).toThrow(/did not export __rrTest/);
    expect(warn).toHaveBeenCalledWith("revscope:", "Multiple stub functions found");
  });

  it("reads projectName from stub config.project", () => {
    const env = load({
      projectName: "fromname",
      config: { hostname: "h.example" },
    });
    expect(env.rr.getProjectName()).toBe("fromname");
    expect(env.window.dispatch.projectName).toBe("fromname");
    expect(env.window.dispatch.c).toEqual({ hostname: "h.example", project: "fromname" });
    expect(env.requests[0].url).toContain("/fromname");
    expect(env.requests[0].body).not.toContain("pid=");
    expect(env.rr.getTrackedHostname()).toBe("h.example");
  });

  it("uses config.name as the window function", () => {
    const env = load({
      projectName: "acme",
      config: { name: "rs" },
    });
    expect(typeof env.window.rs).toBe("function");
    expect(env.window.rs.projectName).toBe("acme");
    expect(env.window.rs.c).toEqual({ name: "rs", project: "acme" });
    expect(Object.hasOwn(env.window, "dispatch")).toBe(false);
    expect(env.requests[0].url).toContain("/acme");
  });

  it("accepts legacy dispatch_{project} stubs without config.project", () => {
    const env = load({
      installStub: false,
      beforeLoad(windowObj) {
        const queue = [];
        function stub() {
          queue.push([].slice.call(arguments));
        }
        stub.q = queue;
        stub.a = queue;
        stub.c = { hostname: "h.example" };
        windowObj.dispatch_fromname = stub;
      },
    });
    expect(env.rr.getProjectName()).toBe("fromname");
    expect(env.window.dispatch_fromname.projectName).toBe("fromname");
    expect(env.rr.getTrackedHostname()).toBe("h.example");
    expect(env.requests[0].url).toContain("/fromname");
  });

  it("finds the executing script through the IE9 readyState fallback", () => {
    const env = load({ currentScript: false });
    expect(env.requests[0].url).toContain("127.0.0.1:4320/demo");
  });

  it.each([
    ["https://assets.example.com", "https://assets.example.com/demo"],
    ["https://assets.example.com/", "https://assets.example.com/demo"],
    ["assets.example.com", "https://assets.example.com/demo"],
    ["assets.example.com/", "https://assets.example.com/demo"],
    ["www.example.com/site-assets", "https://www.example.com/site-assets/demo"],
    ["http://localhost:4320", "http://localhost:4320/demo"],
    ["/site-assets", "/site-assets/demo"],
    ["/site-assets/", "/site-assets/demo"],
  ])("uses configured ingest base URL %j", (url, expected) => {
    const env = load({ config: { url } });
    expect(env.requests[0].url).toBe(expected);
  });

  it("reads config from the stub .c object before initialization", () => {
    const env = load({
      config: {
        hostname: "queued.example",
        metadata: { source: "stub" },
      },
    });
    expect(env.rr.getTrackedHostname()).toBe("queued.example");
    expect(env.rr.getPageMetadata()).toEqual({ source: "stub" });
    expect(env.window.dispatch.c).toEqual({
      hostname: "queued.example",
      metadata: { source: "stub" },
      project: "demo",
    });
  });

  it("rejects stubs without a plain-object .c config", () => {
    const warn = vi.fn();
    expect(() =>
      load({
        warn,
        installStub: false,
        beforeLoad(windowObj) {
          const queue = [];
          function stub() {
            queue.push([].slice.call(arguments));
          }
          stub.q = queue;
          stub.a = queue;
          windowObj.dispatch = stub;
        },
      }),
    ).toThrow(/did not export __rrTest/);
    expect(warn).toHaveBeenCalledWith("revscope:", "Project name missing; install the stub script");
  });

  it("treats config dispatched after load as an unknown action", () => {
    const warn = vi.fn();
    const env = load({ warn });
    expect(env.dispatch("config", { hostname: "late.example" })).toBeUndefined();
    expect(env.rr.getTrackedHostname()).toBe("example.com");
    expect(warn).toHaveBeenCalledWith("revscope:", "unknown action: config");
  });

  it("deduplicates later stubs and tracker loads through function properties", () => {
    const env = load();
    const requestCount = env.requests.length;
    const dispatch = env.window.dispatch;

    const duplicateStub = env.installStub();
    expect(duplicateStub).toBe(dispatch);
    expect(duplicateStub.q).toEqual([]);
    expect(duplicateStub.c).toEqual({ project: "demo" });
    expect(duplicateStub.projectName).toBe("demo");

    env.runTracker();
    expect(env.window.dispatch).toBe(dispatch);
    expect(env.requests).toHaveLength(requestCount);
  });

  it("trackEvent normalizes names and enqueues", () => {
    const env = load();
    env.rr.trackEvent("Buy Now!!!");
    const bodies = env.requests.map((r) => r.body || r.url || "").join("\n");
    const queued = env.rr.getBatchQueue();
    const sawEvent =
      bodies.includes("Buy-Now") ||
      bodies.includes("ev=Buy-Now") ||
      queued.some((p) => p.ev === "Buy-Now");
    expect(sawEvent).toBe(true);
  });

  it("trackEvent rejects non-string names", () => {
    const warn = vi.fn();
    const env = load({ warn });
    const callback = vi.fn();
    env.rr.trackEvent(123, { callback });
    expect(warn).toHaveBeenCalled();
    expect(callback).toHaveBeenCalled();
  });

  it("metadata command merges page metadata and sends only changes", () => {
    const env = load({
      config: {
        projectName: "demo",
        metadata: { plan: "free", stable: "same" },
      },
    });

    env.dispatch("metadata", { plan: "pro", region: "us" });
    env.dispatch("metadata", { plan: "pro", region: "eu" });

    const updates = env.rr.getBatchQueue().filter((payload) => payload.ty === "metadata");
    expect(updates).toHaveLength(2);
    expect(JSON.parse(updates[0].meta)).toEqual({ plan: "pro", region: "us" });
    expect(JSON.parse(updates[1].meta)).toEqual({ region: "eu" });
    expect(env.rr.getPageMetadata()).toEqual({
      plan: "pro",
      stable: "same",
      region: "eu",
    });
  });

  it("pageviews send metadata changes since the last metadata payload", () => {
    const env = load({
      config: {
        projectName: "demo",
        metadata: (context) => ({
          plan: context.path === "/" ? "free" : "pro",
          stable: "same",
        }),
      },
    });

    env.rr.trackPageview(true, "/account");
    env.rr.trackPageview(true, "/billing");

    const pageviews = env.rr.getBatchQueue().filter((payload) => payload.ty === "pageview");
    expect(pageviews).toHaveLength(2);
    expect(JSON.parse(pageviews[0].meta)).toEqual({ plan: "pro" });
    expect(pageviews[1].meta).toBeUndefined();
  });

  describe("history navigation", () => {
    function queued(env, type) {
      return env.rr.getBatchQueue().filter((payload) => payload.ty === type);
    }

    it.each(["pushState", "replaceState"])("%s to a new path records a pageview", (method) => {
      const env = load();
      const landingPageId = env.rr.getPageId();

      env.window.history[method](null, "", "/continue");

      expect(env.window.location.pathname).toBe("/continue");
      const pageviews = queued(env, "pageview");
      expect(pageviews).toHaveLength(1);
      expect(pageviews[0].p).toBe("/continue");
      expect(pageviews[0].pg).toBe(env.rr.getPageId());
      expect(env.rr.getPageId()).not.toBe(landingPageId);
    });

    it("replaceState credits engagement so far to the page being left", () => {
      const env = load();
      const landingPageId = env.rr.getPageId();

      env.window.history.replaceState(null, "", "/continue");

      const engagement = queued(env, "engagement");
      expect(engagement).toHaveLength(1);
      expect(engagement[0].pg).toBe(landingPageId);
    });

    it("replaceState calls the browser's method with its arguments and result", () => {
      const replaceState = vi.fn(() => "result");
      const history = { pushState: vi.fn(), replaceState };
      const env = load({ history });

      const result = env.window.history.replaceState({ step: 1 }, "", "/continue");

      expect(replaceState).toHaveBeenCalledExactlyOnceWith({ step: 1 }, "", "/continue");
      expect(replaceState.mock.contexts[0]).toBe(history);
      expect(result).toBe("result");
    });

    it.each([
      ["a state-only update", { step: 1 }, "", undefined],
      ["the same URL", null, "", "/"],
      ["a query change", null, "", "/?tab=pages"],
      ["a hash change", null, "", "/#section"],
    ])("replaceState with %s records no pageview", (_name, state, title, url) => {
      const env = load();
      const pageId = env.rr.getPageId();

      env.window.history.replaceState(state, title, url);

      expect(queued(env, "pageview")).toHaveLength(0);
      expect(env.rr.getPageId()).toBe(pageId);
    });

    it("replaceState with a hash change records a pageview in hash mode", () => {
      const env = load({ config: { projectName: "demo", mode: "hash" } });

      env.window.history.replaceState(null, "", "/#/continue");

      expect(queued(env, "pageview").map((payload) => payload.p)).toEqual(["/#/continue"]);
    });

    it("replaceState then pushState back records both screens", () => {
      const env = load();

      env.window.history.replaceState(null, "", "/continue");
      env.window.history.pushState(null, "", "/");

      expect(queued(env, "pageview").map((payload) => payload.p)).toEqual(["/continue", "/"]);
    });

    it("replaceState to an ignored page records no pageview", () => {
      const env = load({
        config: { projectName: "demo", ignorePages: ["/secret"] },
        warn: vi.fn(),
      });

      env.window.history.replaceState(null, "", "/secret");

      expect(queued(env, "pageview")).toHaveLength(0);
    });

    it("replaceState on an ignored page warns about it once", () => {
      const warn = vi.fn();
      const env = load({
        config: { projectName: "demo", ignorePages: ["/secret"] },
        location: { pathname: "/secret", href: "https://example.com/secret" },
        warn,
      });
      const ignoreWarnings = () =>
        warn.mock.calls.filter((args) => args.join(" ").includes("Ignoring /secret"));

      env.window.history.replaceState({ y: 1 }, "");
      env.window.history.replaceState({ y: 2 }, "");

      expect(ignoreWarnings()).toHaveLength(1);
    });

    it("warns again when returning to an ignored page", () => {
      const warn = vi.fn();
      const env = load({ config: { projectName: "demo", ignorePages: ["/secret"] }, warn });
      const ignoreWarnings = () =>
        warn.mock.calls.filter((args) => args.join(" ").includes("Ignoring /secret"));

      env.window.history.replaceState(null, "", "/secret");
      env.window.history.replaceState(null, "", "/public");
      env.window.history.replaceState(null, "", "/secret");

      expect(ignoreWarnings()).toHaveLength(2);
    });

    describe("while the tab is hidden", () => {
      let now;

      beforeEach(() => {
        now = 1_000_000_000_000;
        vi.spyOn(Date, "now").mockImplementation(() => now);
      });

      afterEach(() => {
        vi.restoreAllMocks();
      });

      function sentEngagementDurations(env) {
        return env.requests
          .flatMap((request) => JSON.parse(new URLSearchParams(request.body).get("items") || "[]"))
          .filter((item) => item.ty === "engagement")
          .map((item) => item.d);
      }

      function setHidden(env, hidden) {
        env.window.document.hidden = hidden;
        env.window.dispatchEvent(new env.window.Event("visibilitychange"));
      }

      function loadOnPage({ pagehide = true } = {}) {
        // Without sendBeacon the flush falls back to XHR, whose body the harness keeps readable.
        const env = load({ navigator: { sendBeacon: undefined } });
        if (pagehide) {
          env.window.onpagehide = null;
        }
        env.rr.setPageEngagementStartedAt(now);
        env.rr.setLastScrollActivityAt(now);
        return env;
      }

      it("replaceState splits engagement around the hidden period", () => {
        const env = loadOnPage();

        now += 10_000;
        setHidden(env, true);
        now += 60_000;
        env.window.history.replaceState(null, "", "/login");
        now += 30_000;
        setHidden(env, false);
        now += 20_000;
        env.window.history.pushState(null, "", "/next");

        const durations = queued(env, "engagement").map((payload) => payload.d);
        expect(durations).toEqual([10, 20]);
      });

      it.each([
        ["past the idle threshold", 10_000, 200_000, [10, 20]],
        ["crossing the idle threshold with visible time", 50_000, 80_000, [50, 20]],
      ])(
        "replaceState doesn't count hidden time as idle %s",
        (_name, visible, hidden, durations) => {
          const env = loadOnPage();

          now += visible;
          setHidden(env, true);
          now += hidden;
          env.window.history.replaceState(null, "", "/login");
          now += 30_000;
          setHidden(env, false);
          now += 20_000;
          env.window.history.pushState(null, "", "/next");

          expect(queued(env, "engagement").map((payload) => payload.d)).toEqual(durations);
        },
      );

      it("pagehide doesn't count hidden time as idle", () => {
        const env = loadOnPage();

        now += 10_000;
        setHidden(env, true);
        now += 200_000;
        env.window.dispatchEvent(new env.window.Event("pagehide"));

        expect(sentEngagementDurations(env)).toEqual([10]);
      });

      function loadHidden() {
        return load({
          navigator: { sendBeacon: undefined },
          beforeLoad(window) {
            window.document.hidden = true;
          },
        });
      }

      it("replaceState in a tab loaded hidden sends no engaged time", () => {
        const env = loadHidden();

        now += 5_000;
        env.window.history.replaceState(null, "", "/login");
        now += 30_000;
        setHidden(env, false);
        now += 20_000;
        env.window.history.pushState(null, "", "/next");

        expect(queued(env, "engagement").map((payload) => payload.d)).toEqual([0, 20]);
      });

      it("showing a tab loaded hidden counts only the visible time", () => {
        const env = loadHidden();

        now += 30_000;
        setHidden(env, false);
        now += 7_000;
        env.window.history.pushState(null, "", "/next");

        expect(queued(env, "engagement").map((payload) => payload.d)).toEqual([7]);
      });

      it("pagehide doesn't credit the hidden period", () => {
        const env = loadOnPage();

        now += 10_000;
        setHidden(env, true);
        now += 60_000;
        env.window.dispatchEvent(new env.window.Event("pagehide"));

        expect(sentEngagementDurations(env)).toEqual([10]);
      });

      it("browsers without pagehide don't subtract an earlier hidden period again", () => {
        const env = loadOnPage({ pagehide: false });

        now += 10_000;
        setHidden(env, true);
        now += 5_000;
        setHidden(env, false);
        now += 7_000;
        setHidden(env, true);

        expect(sentEngagementDurations(env)).toEqual([10, 7]);
      });
    });

    it("replaceState records nothing while tracking is disabled", () => {
      const env = load();
      env.rr.flushTrackingQueue(false);
      env.dispatch("set_consent", "disabled");

      env.window.history.replaceState(null, "", "/continue");

      expect(queued(env, "pageview")).toHaveLength(0);
    });
  });

  it("setConsent writes cookie and clears session when cookies are denied", () => {
    const env = load({
      cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
    });
    expect(env.rr.getConsent()).toBe("granted");
    // Initial pageview XHR may refresh the session id from the server response.
    expect(env.rr.getSessionId()).toBeTruthy();

    env.rr.setConsent("denied");
    expect(env.rr.getConsent()).toBe("denied");
    expect(env.cookies.entries().rcdemo).toBe("0");
    expect(env.rr.getSessionId()).toBeUndefined();
    expect(env.cookies.entries().rsdemo).toBeUndefined();
  });

  it("disabled consent forgets the visitor then suppresses all tracking", () => {
    const callback = vi.fn();
    const env = load({
      cookies: { rsdemo: "0000000000003039", rcdemo: "1" },
    });
    // Clear the context hit queued by storage_allowed on the initial pageview.
    env.rr.flushTrackingQueue(false);
    const requestsBefore = env.requests.length;
    const pageId = env.rr.getPageId();
    const sessionId = env.rr.getSessionId();

    env.dispatch("event", "queued-before-disable");
    expect(env.rr.getBatchQueue()).toHaveLength(1);
    env.dispatch("set_consent", "disabled");
    env.dispatch("event", "after-disable", { callback });
    env.rr.trackPageview(true, "/after-disable");
    env.dispatch("metadata", { plan: "private" });

    expect(env.rr.getConsent()).toBe("disabled");
    expect(env.cookies.entries().rcdemo).toBe("2");
    expect(env.cookies.entries().rsdemo).toBeUndefined();
    expect(env.rr.getSessionId()).toBeUndefined();
    expect(env.rr.getBatchQueue()).toHaveLength(0);
    expect(env.requests).toHaveLength(requestsBefore + 1);
    expect(callback).toHaveBeenCalledOnce();

    const request = env.requests[env.requests.length - 1];
    const params = new URLSearchParams(request.body);
    const items = JSON.parse(params.get("items"));
    expect(params.get("s")).toBe(sessionId);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      ty: "forget",
      pg: pageId,
    });
  });

  it("a saved disabled choice suppresses the initial pageview", () => {
    const env = load({
      cookies: { rsdemo: "0000000000003039", rcdemo: "2" },
    });
    let received;

    env.dispatch("consent_status", (status) => {
      received = status;
    });

    expect(env.requests).toHaveLength(0);
    expect(env.rr.getConsent()).toBe("disabled");
    expect(env.rr.getSessionId()).toBeUndefined();
    expect(received).toBe("disabled");
  });

  it("queued disabled consent suppresses initial and queued tracking", () => {
    const env = load({
      queue: [
        ["event", "queued-before-load"],
        ["set_consent", "disabled"],
      ],
    });

    expect(env.requests).toHaveLength(0);
    expect(env.rr.getBatchQueue()).toHaveLength(0);
    expect(env.cookies.entries().rcdemo).toBe("2");
  });

  it("tracking resumes with a fresh pageview after disabled consent changes", () => {
    const env = load({ cookies: { rcdemo: "2" } });

    env.dispatch("set_consent", "denied");

    expect(env.requests).toHaveLength(1);
    expect(env.requests[0].body).toContain("c=false");
    expect(env.rr.getConsent()).toBe("denied");
    expect(env.rr.getPageId()).toBeTruthy();
  });

  it("setConsent rejects values outside the documented string states", () => {
    const warn = vi.fn();
    const env = load({ warn });
    const initialConsent = env.rr.getConsent();

    env.dispatch("set_consent", true);
    env.dispatch("set_consent", "unknown");

    expect(env.rr.getConsent()).toBe(initialConsent);
    expect(
      warn.mock.calls.filter((call) => String(call[1]).startsWith("consent must be granted")),
    ).toHaveLength(2);
  });

  it("global dispatch function handles public actions", () => {
    const env = load();
    env.dispatch("set_consent", "granted");
    expect(env.rr.getConsent()).toBe("granted");
    expect(env.cookies.entries().rcdemo).toBe("1");

    env.dispatch("event", "signup");
    const bodies = env.requests.map((r) => r.body || "").join("\n");
    const queued = env.rr.getBatchQueue();
    const saw = bodies.includes("ev=signup") || queued.some((p) => p.ev === "signup");
    expect(saw).toBe(true);

    env.dispatch("metadata", { plan: "pro" });
    expect(
      env.rr
        .getBatchQueue()
        .some((payload) => payload.ty === "metadata" && JSON.parse(payload.meta).plan === "pro"),
    ).toBe(true);
  });

  it("consent_status is not_required when the server allows session persistence", () => {
    const env = load();
    let received;
    env.dispatch("consent_status", (status) => {
      received = status;
    });
    expect(received).toBe("not_required");
  });

  it("replays a queued consent_status callback", () => {
    let received;
    load({
      queue: [
        [
          "consent_status",
          (status) => {
            received = status;
          },
        ],
      ],
    });
    expect(received).toBe("not_required");
  });

  it("consent_status is required when storage_allowed is false and no session cookie", () => {
    const env = load({
      ingestResponse: {
        visitor_timestamp_id: "00000000499602d2",
        storage_allowed: false,
      },
    });
    let received;
    env.dispatch("consent_status", (status) => {
      received = status;
    });
    expect(received).toBe("required");
    expect(env.cookies.entries().rsdemo).toBeUndefined();
  });

  it("consent_status is not_required when a session cookie already exists", () => {
    const env = load({
      cookies: { rsdemo: "0000000000003039" },
      ingestResponse: {
        visitor_timestamp_id: "0000000000003039",
        storage_allowed: false,
      },
    });
    let received;
    env.dispatch("consent_status", (status) => {
      received = status;
    });
    expect(received).toBe("not_required");
  });

  it("consent_status is denied when a prior consent cookie is 0", () => {
    const env = load({
      cookies: { rcdemo: "0" },
      ingestResponse: {
        visitor_timestamp_id: "00000000499602d2",
        storage_allowed: false,
      },
    });
    let received;
    env.dispatch("consent_status", (status) => {
      received = status;
    });
    expect(received).toBe("denied");
  });

  it("consent_status is granted when a prior consent cookie is 1", () => {
    const env = load({
      cookies: { rcdemo: "1" },
      ingestResponse: {
        visitor_timestamp_id: "00000000499602d2",
        storage_allowed: false,
      },
    });
    let received;
    env.dispatch("consent_status", (status) => {
      received = status;
    });
    expect(received).toBe("granted");
  });

  it("buffers a follow-up event when consent and session cookies are missing", () => {
    const env = load({
      ingestResponse: {
        visitor_timestamp_id: "00000000499602d2",
        storage_allowed: false,
      },
    });
    const requestsBefore = env.requests.length;
    env.dispatch("event", "prompt-flush");
    expect(env.requests.length).toBe(requestsBefore);
    expect(env.rr.getBatchQueue().some((payload) => payload.ev === "prompt-flush")).toBe(true);
  });

  describe("device context", () => {
    const deviceKeys = ["tz", "tch", "sw", "sh", "vw", "vh", "l"];

    function allowingResponse() {
      return {
        visitor_timestamp_id: "00000000499602d2",
        storage_allowed: true,
      };
    }

    function contextHitCount(requests) {
      let count = 0;
      for (const request of requests) {
        const params = new URLSearchParams(request.body || "");
        if (params.get("ty") === "context") {
          count += 1;
        }
        const items = params.get("items");
        if (items) {
          count += JSON.parse(items).filter((item) => item.ty === "context").length;
        }
      }
      return count;
    }

    it("never reads the device for the initial pageview", () => {
      const env = load();
      const params = new URLSearchParams(env.requests[0].body);
      expect(params.get("ty")).toBe("pageview");
      for (const key of deviceKeys) {
        expect(params.get(key)).toBeNull();
      }
    });

    it("stays quiet while the server has not allowed storage", () => {
      const env = load({
        ingestResponse: {
          visitor_timestamp_id: "00000000499602d2",
          storage_allowed: false,
        },
      });
      expect(env.rr.getBatchQueue()).toHaveLength(0);
      expect(contextHitCount(env.requests)).toBe(0);
      expect(env.cookies.entries().rsdemo).toBeUndefined();
    });

    it("follows up with a context hit once the server allows it", () => {
      const env = load({ ingestResponse: allowingResponse() });
      const queued = env.rr.getBatchQueue();
      expect(queued).toHaveLength(1);
      expect(queued[0]).toMatchObject({
        ty: "context",
        pg: env.rr.getPageId(),
        tz: "UTC",
        l: "en-US",
        tch: false,
        sw: 1920,
        sh: 1080,
        vw: 1024,
        vh: 768,
      });
    });

    it("sends the context hit only once per page load", () => {
      const env = load({ ingestResponse: allowingResponse() });
      env.rr.flushTrackingQueue(false);
      env.dispatch("event", "second-hit");
      env.rr.flushTrackingQueue(false);
      expect(contextHitCount(env.requests)).toBe(1);
    });

    it("reads the device and writes the cookie after the visitor grants consent", () => {
      const env = load({
        ingestResponse: {
          visitor_timestamp_id: "00000000499602d2",
          storage_allowed: false,
        },
      });
      expect(env.rr.getBatchQueue()).toHaveLength(0);
      expect(env.cookies.entries().rsdemo).toBeUndefined();

      env.dispatch("set_consent", "granted");

      const contextHit = env.rr.getBatchQueue().find((payload) => payload.ty === "context");
      expect(contextHit).toMatchObject({ ty: "context", tz: "UTC", l: "en-US" });
      expect(env.cookies.entries().rsdemo).toBe("00000000499602d2");
    });

    it("does not read the device after consent is denied", () => {
      const env = load({ ingestResponse: allowingResponse() });
      env.rr.flushTrackingQueue(false);

      env.dispatch("set_consent", "denied");

      expect(env.rr.getBatchQueue().some((payload) => payload.ty === "context")).toBe(false);
    });
  });

  it("get_session passes the canonical server session id to its callback", () => {
    const env = load();
    let received;
    const result = env.dispatch("get_session", (sessionId) => {
      received = sessionId;
      return "must not be returned";
    });
    expect(result).toBeUndefined();
    expect(received).toBe("00000000499602d2");

    env.rr.applyIngestResponse(
      '{"visitor_timestamp_id":"184b6ca73b9aaff2","storage_allowed":false}',
    );
    env.dispatch("get_session", (sessionId) => {
      received = sessionId;
    });
    expect(received).toBe("184b6ca73b9aaff2");
  });

  it("get_session callback falls back to the current page id", () => {
    const env = load();
    env.rr.setConsent("denied");
    let received;

    env.dispatch("get_session", (sessionId) => {
      received = sessionId;
    });
    expect(received).toBe(env.rr.getPageId());
    expect(received).toMatch(/^[0-9a-f]{24}$/);
  });

  it("get_session requires a callback", () => {
    const warn = vi.fn();
    const env = load({ warn });
    expect(env.dispatch("get_session")).toBeUndefined();
    expect(warn).toHaveBeenCalledWith("revscope:", "callback must be a function, not: undefined");
  });

  it("replays a queued get_session callback after the initial pageview", () => {
    let received;
    load({
      queue: [
        [
          "get_session",
          (sessionId) => {
            received = sessionId;
          },
        ],
      ],
    });
    expect(received).toBe("00000000499602d2");
  });

  it("replays queued calls from the stub", () => {
    const env = load({
      queue: [
        ["set_consent", "granted"],
        ["event", "queued-one"],
      ],
    });
    expect(env.rr.getConsent()).toBe("granted");
    const bodies = env.requests.map((r) => r.body || "").join("\n");
    const queued = env.rr.getBatchQueue();
    const saw = bodies.includes("queued-one") || queued.some((p) => p.ev === "queued-one");
    expect(saw).toBe(true);
    expect(env.window.dispatch.projectName).toBe("demo");
    expect(env.window.dispatch.q).toEqual([]);
  });

  it("uses configured hostname override", () => {
    const env = load({
      config: {
        projectName: "demo",
        hostname: "demo.revscope.test",
      },
    });
    expect(env.rr.getTrackedHostname()).toBe("demo.revscope.test");
    expect(env.requests[0].body).toContain("h=demo.revscope.test");
  });

  it("suppresses tracking on localhost and IP hosts without an override", () => {
    const warn = vi.fn();
    expect(() =>
      load({
        location: { hostname: "localhost", host: "localhost" },
        warn: warn,
      }),
    ).toThrow(/did not export __rrTest/);
    expect(warn).toHaveBeenCalledWith("revscope:", "Local or IP hostname, not tracking");

    warn.mockClear();
    expect(() =>
      load({
        location: { hostname: "127.0.0.1", host: "127.0.0.1" },
        warn: warn,
      }),
    ).toThrow(/did not export __rrTest/);
    expect(warn).toHaveBeenCalledWith("revscope:", "Local or IP hostname, not tracking");
  });

  it("tracks on localhost when hostname override is reportable", () => {
    const env = load({
      location: { hostname: "localhost", host: "localhost" },
      config: { hostname: "example.com" },
    });
    expect(env.rr.getTrackedHostname()).toBe("example.com");
    expect(env.requests[0].body).toContain("h=example.com");
  });

  it("accepts data-hostname override on localhost", () => {
    const env = load({
      location: { hostname: "localhost", host: "localhost" },
      scriptAttrs: { "data-hostname": "demo.revscope.test" },
    });
    expect(env.rr.getTrackedHostname()).toBe("demo.revscope.test");
    expect(env.requests[0].body).toContain("h=demo.revscope.test");
  });
});

describe("production minify strips test exports", () => {
  it("RR_TEST markers remove __rrTest with zero size delta", async () => {
    const { readFileSync } = await import("node:fs");
    const { minify } = await import("uglify-js");
    const { dirname, join } = await import("node:path");
    const { fileURLToPath } = await import("node:url");

    const source = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), "..", "tracker.js"),
      "utf8",
    );
    const stripped = source.replace(
      /\n?[ \t]*\/\* RR_TEST_START \*\/[\s\S]*?\/\* RR_TEST_END \*\//g,
      "",
    );
    expect(stripped).not.toBe(source);
    expect(stripped).not.toContain("__rrTest");

    const withExports = minify(source, {
      compress: { arrows: false },
      mangle: true,
    });
    const withoutExports = minify(stripped, {
      compress: { arrows: false },
      mangle: true,
    });
    expect(withExports.error).toBeUndefined();
    expect(withoutExports.error).toBeUndefined();
    expect(withoutExports.code).not.toContain("__rrTest");
    // Keeping the export block in source would bloat the bundle; stripping
    // must match a build that never had the block.
    expect(withoutExports.code.length).toBeLessThan(withExports.code.length);
  });
});
