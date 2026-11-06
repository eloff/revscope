import { render, renderHook, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";
import PeriodComparisonNote from "./PeriodComparisonNote";

const NOW = new Date("2026-03-15T12:00:00Z");
const DAY = 86_400;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(() => vi.useRealTimers());

function renderState(timeZone?: () => string) {
  return renderHook(useDashboardState, {
    wrapper: (props) => (
      <DashboardStateProvider timeZone={timeZone}>{props.children}</DashboardStateProvider>
    ),
  });
}

describe("DashboardStateProvider", () => {
  it("throws outside the provider", () => {
    expect(() => renderHook(useDashboardState)).toThrow(/within DashboardStateProvider/);
  });

  it("starts on the last 30 days once mounted, compared with the 30 days before", () => {
    const { result } = renderState();
    expect(result.rangeReady()).toBe(true);
    // Thirty days back plus today, as dateRange.test.ts pins down.
    expect(result.dayCount()).toBe(31);
    expect(result.previousRange()).toEqual({
      start: result.range().start - 30 * DAY,
      end: result.range().start,
    });
    expect(result.comparisonNote()).toMatch(/^Percent change vs the prior 30 days/);
  });

  it("opens on a range carried over from another project", () => {
    const initialRange = { start: 1_700_000_000, end: 1_700_000_000 + 7 * DAY };
    const { result } = renderHook(useDashboardState, {
      wrapper: (props) => (
        <DashboardStateProvider initialRange={initialRange}>
          {props.children}
        </DashboardStateProvider>
      ),
    });
    expect(result.range()).toEqual(initialRange);
  });

  it("realigns the default range when the project's zone arrives", () => {
    const [zone, setZone] = createSignal("UTC");
    const { result } = renderState(zone);
    const utcStart = result.range().start;
    setZone("Asia/Tokyo");
    expect(result.timeZone()).toBe("Asia/Tokyo");
    expect(result.range().start).toBe(utcStart - 9 * 3600);
  });

  it("keeps a range the user picked when the zone changes", () => {
    const [zone, setZone] = createSignal("UTC");
    const { result } = renderState(zone);
    const picked = { start: 1_700_000_000, end: 1_700_000_000 + 7 * DAY };
    result.setRange(picked);
    setZone("Asia/Tokyo");
    expect(result.range()).toEqual(picked);
    expect(result.dayCount()).toBe(7);
  });

  it("adds, removes and clears segment filters", () => {
    const { result } = renderState();
    result.addCountryFilter("us", "United States");
    result.addSourceFilter({ kind: "direct", name: "Direct" });
    result.addPageFilter({ id: 3, path: "/pricing" } as never);
    expect(result.segments().countries).toEqual([{ code: "US", name: "United States" }]);
    expect(result.segments().sources).toHaveLength(1);

    result.removeCountryFilter("US");
    expect(result.segments().countries).toEqual([]);
    result.clearSegments();
    expect(result.segments().sources).toEqual([]);
    expect(result.segments().pages).toEqual([]);
  });
});

describe("PeriodComparisonNote", () => {
  it("renders the context's note with change arrows", () => {
    render(() => (
      <DashboardStateProvider>
        <PeriodComparisonNote />
      </DashboardStateProvider>
    ));
    const note = screen.getByText(/vs the prior 30 days/);
    expect(note).toHaveTextContent(/^Percent change ▲ ▼ vs the prior 30 days \(.+\), scaled/);
  });
});
