import { afterEach, describe, expect, it } from "vitest";
import {
  readDashboardRoute,
  rewriteLegacyTabUrl,
  setPagesModeParams,
  setPeopleModeParams,
} from "./dashboardRoute";
import { campaignSourceKey } from "./revenueSourcesLogic";

function at(search: string) {
  window.history.replaceState(null, "", `/dashboard${search}`);
}

afterEach(() => at(""));

describe("readDashboardRoute", () => {
  it("opens the tab a drill-down param implies when none is named", () => {
    at("?cc=de");
    expect(readDashboardRoute()).toMatchObject({ tab: "audience", countryCode: "DE" });
    at("?source=campaign:7");
    expect(readDashboardRoute()).toMatchObject({
      tab: "revenue_sources",
      revenueSourceKey: campaignSourceKey(7),
    });
    at("?landing_path_id=3");
    expect(readDashboardRoute()).toMatchObject({
      tab: "pages",
      pagesMode: "landing",
      landingPathId: 3,
    });
    at("?filter=pageview&path_id=4");
    expect(readDashboardRoute()).toMatchObject({
      tab: "visitors",
      visitorFilter: { kind: "pageview", pathId: 4 },
    });
  });

  it("drops drill-downs that belong to another tab or mode", () => {
    at("?tab=events&cc=DE&path_id=4&source=direct");
    expect(readDashboardRoute()).toMatchObject({
      tab: "events",
      countryCode: null,
      pathId: null,
      revenueSourceKey: null,
      visitorFilter: null,
    });
    at("?tab=visitors&people=customers&filter=pageview&path_id=4");
    expect(readDashboardRoute()).toMatchObject({ peopleMode: "customers", visitorFilter: null });
  });

  it("rejects malformed drill-down values", () => {
    at("?tab=audience&cc=DEU");
    expect(readDashboardRoute().countryCode).toBeNull();
    at("?tab=pages&path_id=-2");
    expect(readDashboardRoute().pathId).toBeNull();
    at("?tab=revenue_sources&source=campaign:x");
    expect(readDashboardRoute().revenueSourceKey).toBeNull();
  });

  it("maps legacy tabs onto the combined tabs", () => {
    at("?tab=conversions");
    expect(readDashboardRoute()).toMatchObject({ tab: "visitors", peopleMode: "customers" });
    at("?tab=landing_pages");
    expect(readDashboardRoute()).toMatchObject({ tab: "pages", pagesMode: "landing" });
  });
});

describe("rewriteLegacyTabUrl", () => {
  it("rewrites only legacy tab URLs", () => {
    at("?tab=conversions&x=1");
    expect(rewriteLegacyTabUrl()).toBe(true);
    expect(window.location.search).toBe("?tab=visitors&x=1&people=customers");
    at("?tab=events");
    expect(rewriteLegacyTabUrl()).toBe(false);
    expect(window.location.search).toBe("?tab=events");
  });
});

describe("mode params", () => {
  it("drops the other people mode's drill-downs", () => {
    const params = new URLSearchParams("filter=pageview&path_id=4&payment_id=9");
    setPeopleModeParams(params, "customers");
    expect(params.toString()).toBe("payment_id=9&tab=visitors&people=customers");
    setPeopleModeParams(params, "visitors");
    expect(params.toString()).toBe("tab=visitors");
  });

  it("drops the other pages mode's drill-downs", () => {
    const params = new URLSearchParams("path_id=4&landing_path_id=5");
    setPagesModeParams(params, "landing");
    expect(params.toString()).toBe("landing_path_id=5&tab=pages&pages=landing");
    setPagesModeParams(params, "pages");
    expect(params.toString()).toBe("tab=pages");
  });
});
