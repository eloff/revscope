import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { civilToUnix } from "../../lib/timezone";
import DashboardFilters from "./DashboardFilters";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";

const NY = "America/New_York";
const midnight = (year: number, month: number, day: number) =>
  civilToUnix(NY, { year, month, day, hour: 0, minute: 0, second: 0 });

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-04-20T16:00:00Z"));
});

afterEach(() => vi.useRealTimers());

function renderFilters(
  props: Parameters<typeof DashboardFilters>[0] = {},
  initialRange?: { start: number; end: number },
) {
  let state!: ReturnType<typeof useDashboardState>;
  const Capture = () => {
    state = useDashboardState();
    return null;
  };
  render(() => (
    <DashboardStateProvider timeZone={() => NY} initialRange={initialRange}>
      <Capture />
      <DashboardFilters showAccount={false} {...props} />
    </DashboardStateProvider>
  ));
  return () => state;
}

describe("DashboardFilters", () => {
  it("links the project's timezone to settings", () => {
    renderFilters();
    expect(screen.getByRole("link", { name: "America/New_York (UTC-4)" })).toHaveAttribute(
      "href",
      "/settings#timezone",
    );
  });

  it("steps back a whole period and can't step past today", async () => {
    const state = renderFilters();
    state().setRange({ start: midnight(2026, 4, 1), end: midnight(2026, 4, 11) });
    expect(screen.getByRole("button", { name: "Next period" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Previous period" }));
    expect(state().range()).toEqual({ start: midnight(2026, 3, 22), end: midnight(2026, 4, 1) });

    state().setRange({ start: midnight(2026, 4, 11), end: midnight(2026, 4, 21) });
    expect(screen.getByRole("button", { name: "Next period" })).toBeDisabled();
  });

  it("keeps period boundaries on local midnight across a DST change", async () => {
    const state = renderFilters();
    // March 2026 holds the spring-forward day, so it is an hour short of 31 × 24h.
    state().setRange({ start: midnight(2026, 3, 1), end: midnight(2026, 4, 1) });
    await userEvent.click(screen.getByRole("button", { name: "Previous period" }));
    expect(state().range()).toEqual({ start: midnight(2026, 1, 29), end: midnight(2026, 3, 1) });
    await userEvent.click(screen.getByRole("button", { name: "Next period" }));
    expect(state().range()).toEqual({ start: midnight(2026, 3, 1), end: midnight(2026, 4, 1) });
  });

  it("caps the next period at the end of today", async () => {
    const state = renderFilters();
    state().setRange({ start: midnight(2026, 4, 10), end: midnight(2026, 4, 17) });
    await userEvent.click(screen.getByRole("button", { name: "Next period" }));
    expect(state().range()).toEqual({ start: midnight(2026, 4, 17), end: midnight(2026, 4, 21) });
  });

  it("slides a sub-day window by its length", async () => {
    const state = renderFilters();
    const now = Date.parse("2026-04-20T16:00:00Z") / 1000;
    state().setRange({ start: now - 86_400, end: now });
    await userEvent.click(screen.getByRole("button", { name: "Previous period" }));
    expect(state().range()).toEqual({ start: now - 2 * 86_400, end: now - 86_400 });
  });

  it("disables the range controls when the tab uses a fixed window", () => {
    renderFilters({ dateRangeDisabled: true });
    expect(screen.getByRole("button", { name: "Previous period" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Next period" })).toBeDisabled();
  });

  it("lists active segment filters and removes them", async () => {
    const state = renderFilters();
    state().addCountryFilter("DE", "Germany");
    state().addPageFilter({ id: 4, path: "/pricing" } as never);
    await userEvent.click(
      await screen.findByRole("button", { name: "Remove filter: Country: Germany" }),
    );
    expect(state().segments().countries).toEqual([]);
    expect(screen.getByRole("button", { name: /Remove filter: Page/ })).toBeInTheDocument();
  });

  it("applies a preset from the calendar", async () => {
    const state = renderFilters();
    await userEvent.click(screen.getAllByRole("textbox")[0] as HTMLElement);
    await userEvent.click(await screen.findByRole("option", { name: "Last 7 Days" }));
    expect(state().range()).toEqual({ start: midnight(2026, 4, 13), end: midnight(2026, 4, 21) });
  });

  it("marks the preset a carried-over range matches", async () => {
    renderFilters({}, { start: midnight(2026, 4, 19), end: midnight(2026, 4, 20) });
    await userEvent.click(screen.getAllByRole("textbox")[0] as HTMLElement);
    expect(await screen.findByRole("option", { name: "Yesterday" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("option", { name: "Last 30 Days" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });
});
