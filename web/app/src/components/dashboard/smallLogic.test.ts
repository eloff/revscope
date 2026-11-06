import { describe, expect, it } from "vitest";
import type { CampaignListItem } from "../../lib/campaignsApi";
import { campaignSubtitle, campaignTitle, paramLabel } from "./campaignLabels";
import {
  BAR_METRIC_KEYS,
  createDefaultChartVisibility,
  isMetricChartActive,
  toggleChartMetric,
} from "./chartMetrics";
import { formatSourceConversionCell, formatWilsonScoreInterval } from "./conversionConfidence";
import { hoverCardPortalMount } from "./hoverCardPortal";

function campaign(overrides: Partial<CampaignListItem> = {}): CampaignListItem {
  return {
    source: "",
    medium: "",
    campaign: "",
    rules: [],
    ...overrides,
  } as CampaignListItem;
}

describe("campaign labels", () => {
  it("maps rule keys to their UTM names", () => {
    expect(paramLabel("source")).toBe("utm_source");
    expect(paramLabel("ref")).toBe("ref");
    expect(paramLabel("gclid")).toBe("gclid");
  });

  it.each([
    [{ display_name: "Launch", campaign: "spring" }, "Launch"],
    [{ campaign: "spring" }, "spring"],
    [{ network: "google" }, "google ads"],
    [{ network: "generic" }, "Unnamed campaign"],
    [{}, "Unnamed campaign"],
  ])("titles %j as %s", (overrides, expected) => {
    expect(campaignTitle(campaign(overrides))).toBe(expected);
  });

  it.each([
    [{ rules: [{ param_key: "ref", param_value: "x" }] }, "Custom rule"],
    [{ source: "news", medium: "email" }, "news · email"],
    [{ network: "meta", source: "fb" }, "meta click id · fb"],
    [{ network: "generic" }, "UTM campaign"],
  ])("subtitles %j as %s", (overrides, expected) => {
    expect(campaignSubtitle(campaign(overrides))).toBe(expected);
  });
});

describe("conversion confidence", () => {
  it("formats a Wilson interval, or a dash without one", () => {
    expect(formatWilsonScoreInterval({ lower: 0.0123, upper: 0.0456 })).toBe("1.23 - 4.56%");
    expect(formatWilsonScoreInterval({ lower: 0.1, upper: 0.2 }, 0)).toBe("10 - 20%");
    expect(formatWilsonScoreInterval(null)).toBe("—");
  });

  it("shows a measured zero rate as 0% and a missing interval as a dash", () => {
    expect(formatSourceConversionCell(0, { lower: 0, upper: 0.1 })).toBe("0%");
    expect(formatSourceConversionCell(0, undefined)).toBe("—");
    expect(formatSourceConversionCell(0.02, { lower: 0.01, upper: 0.03 })).toBe("1.00 - 3.00%");
  });
});

describe("chart metric toggles", () => {
  const defaults = createDefaultChartVisibility();

  it("shows the default line and bar series", () => {
    expect(isMetricChartActive(defaults, "visitors")).toBe(true);
    expect(isMetricChartActive(defaults, "bounce_rate")).toBe(false);
    expect(isMetricChartActive(defaults, "mrr")).toBe(true);
    expect(isMetricChartActive(defaults, "churn_mrr")).toBe(false);
    expect(isMetricChartActive(defaults, "total_revenue")).toBe(true);
  });

  it("keeps one line metric at a time and can hide it", () => {
    const bounce = toggleChartMetric(defaults, "bounce_rate");
    expect(bounce.lineMetric).toBe("bounce_rate");
    expect(isMetricChartActive(bounce, "visitors")).toBe(false);
    expect(toggleChartMetric(bounce, "bounce_rate").lineMetric).toBeNull();
  });

  it("hides every bar series with the revenue master and restores the defaults", () => {
    const withChurn = toggleChartMetric(defaults, "churn_mrr");
    const off = toggleChartMetric(withChurn, "total_revenue");
    expect(isMetricChartActive(off, "mrr")).toBe(false);
    expect(isMetricChartActive(off, "churn_mrr")).toBe(false);
    const on = toggleChartMetric(off, "total_revenue");
    expect(on.barSeries).toEqual(defaults.barSeries);
  });

  it.each(["mrr", "churn_mrr"] as const)(
    "shows only %s when it is picked while revenue is hidden",
    (key) => {
      const off = toggleChartMetric(defaults, "total_revenue");
      const picked = toggleChartMetric(off, key);
      expect(picked.barEnabled).toBe(true);
      for (const barKey of BAR_METRIC_KEYS) {
        expect(isMetricChartActive(picked, barKey)).toBe(barKey === key);
      }
      expect(isMetricChartActive(toggleChartMetric(picked, key), key)).toBe(false);
    },
  );

  it("does not mutate its input", () => {
    const snapshot = structuredClone(defaults);
    toggleChartMetric(defaults, "mrr");
    toggleChartMetric(defaults, "total_revenue");
    expect(defaults).toEqual(snapshot);
  });
});

describe("hoverCardPortalMount", () => {
  it("creates one body-level mount and reuses it", () => {
    const mount = hoverCardPortalMount();
    expect(mount?.parentElement).toBe(document.body);
    expect(mount?.style.zIndex).toBe("10000");
    expect(hoverCardPortalMount()).toBe(mount);
  });
});
