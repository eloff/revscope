import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_EVENTS_SORT, type EventRow, type EventsResponse } from "../../lib/eventsApi";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import Events, { type EventsProps } from "./Events";

const none = { ratio: null };

function event(overrides: Partial<EventRow>): EventRow {
  return {
    event_name_id: 1,
    event_name: "signup",
    path_id: 10,
    path: "/pricing",
    hostname: "example.com",
    visitors: 250,
    revenue: 12_000,
    conversions: 5,
    conversion_rate: 0.02,
    conversion_lower_bound: 0.01,
    conversion_ci: { lower: 0.01, upper: 0.04 },
    revenue_change: none,
    visitors_change: none,
    conversions_change: none,
    conversion_rate_change: none,
    ...overrides,
  };
}

const data: EventsResponse = {
  events: [
    event({}),
    event({ event_name_id: 2, event_name: "revscope-outbound-click", path_id: 11, path: "/docs" }),
  ],
  offset: 0,
  limit: 200,
  next_offset: 200,
};

function renderEvents(overrides: Partial<EventsProps> = {}) {
  const props: EventsProps = {
    data,
    loading: false,
    error: "",
    sort: DEFAULT_EVENTS_SORT,
    onSortChange: vi.fn(),
    onPageChange: vi.fn(),
    ...overrides,
  };
  render(() => (
    <DashboardStateProvider>
      <Events {...props} />
    </DashboardStateProvider>
  ));
  return props;
}

describe("Events", () => {
  it("lists each event on its page with revenue and conversion interval", () => {
    renderEvents();
    expect(screen.getByText("revscope-outbound-click")).toBeInTheDocument();
    expect(screen.getAllByText("example.com/pricing").length).toBeGreaterThan(0);
    expect(screen.getAllByText("$12").length).toBeGreaterThan(0);
    expect(screen.getAllByText("1.00 - 4.00%").length).toBeGreaterThan(0);
    expect(screen.getByText("Showing 1-2 event rows")).toBeInTheDocument();
  });

  it("starts text columns ascending and numbers descending", async () => {
    const props = renderEvents();
    await userEvent.click(screen.getByRole("button", { name: "Event" }));
    await userEvent.click(screen.getByRole("button", { name: "Visitors" }));
    await userEvent.click(screen.getByRole("button", { name: /^Revenue/ }));
    expect(props.onSortChange).toHaveBeenNthCalledWith(1, { column: "event", direction: "asc" });
    expect(props.onSortChange).toHaveBeenNthCalledWith(2, {
      column: "visitors",
      direction: "desc",
    });
    expect(props.onSortChange).toHaveBeenNthCalledWith(3, { column: "revenue", direction: "asc" });
  });

  it("drills into the page and the visitors who triggered the event", async () => {
    const onPathSelect = vi.fn();
    const onVisitorsSelect = vi.fn();
    renderEvents({ onPathSelect, onVisitorsSelect });
    await userEvent.click(
      screen.getByRole("button", { name: "View details for example.com/docs in Pages" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "View visitors for signup on example.com/pricing" }),
    );
    expect(onPathSelect).toHaveBeenCalledWith(11);
    expect(onVisitorsSelect).toHaveBeenCalledWith({ kind: "event", eventNameId: 1, pathId: 10 });
  });

  it("filters the dashboard by an event and marks active filters", async () => {
    const onEventFilter = vi.fn();
    renderEvents({ onEventFilter, activeEventFilterKeys: new Set(["2"]) });
    expect(
      screen.getByRole("button", { name: "Filtering by revscope-outbound-click" }),
    ).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "Filter by signup" }));
    expect(onEventFilter).toHaveBeenCalledWith(1, "signup");
  });

  it("pages by offset", async () => {
    const props = renderEvents({ data: { ...data, offset: 200, next_offset: undefined } });
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Previous" }));
    expect(props.onPageChange).toHaveBeenCalledWith(0);
  });

  it("shows loading, empty and error states", () => {
    renderEvents({ data: undefined, loading: true });
    expect(screen.getByText("Loading events…")).toBeInTheDocument();
  });

  it("says when there are no events", () => {
    renderEvents({ data: { events: [], offset: 0, limit: 200 } });
    expect(screen.getByText("No events for this period.")).toBeInTheDocument();
  });

  it("shows the tab error", () => {
    renderEvents({ error: "Events failed." });
    expect(screen.getByText("Events failed.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});
