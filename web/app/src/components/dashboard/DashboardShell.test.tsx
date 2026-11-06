import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { projectOptions } from "../../../../shared/src/projectSelection";
import { mockApi } from "../../../../shared/test/api";
import { signIn, signOut } from "../../../../shared/test/auth";
import { testProject, testSession } from "../../../../shared/test/fixtures";
import { testBucket, testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import { setEventsSort } from "../../lib/dashboardPreferences";
import { DEFAULT_EVENTS_SORT, type EventRow } from "../../lib/eventsApi";
import DashboardShell from "./DashboardShell";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => {
  window.history.replaceState(null, "", "/");
  setEventsSort(DEFAULT_EVENTS_SORT);
});

const READ = "/v2/read/projects/1";
const none = { ratio: null };

const overview = (visitors: number) =>
  testOverview({
    current_metrics: testPeriodMetrics({ visitors }),
    buckets: [testBucket({ visitors })],
  });

function eventRow(name: string): EventRow {
  return {
    event_name_id: 1,
    event_name: name,
    path_id: 5,
    path: "/pricing",
    hostname: "x.io",
    visitors: 10,
    revenue: 0,
    conversions: 0,
    conversion_rate: 0,
    conversion_lower_bound: 0,
    revenue_change: none,
    visitors_change: none,
    conversions_change: none,
    conversion_rate_change: none,
  };
}

function mockShell({ chat = false, visitors = 5, coverageHasData = false } = {}) {
  mockApi("get", "/v2/public/chat/status", { available: chat });
  mockApi("get", "/v2/read/projects/:id/imported-coverage", {
    include_in_reports: true,
    has_data: coverageHasData,
    has_events: false,
    ranges: [],
  });
  return mockApi("get", `${READ}/overview`, overview(visitors));
}

/** The Visitors tile's value (hover cards repeat the label, so go by the tile button). */
function visitorsTile(): string {
  const tile = screen
    .getAllByRole("button")
    .find(
      (button) =>
        button.querySelector(":scope > div > div:first-child")?.textContent === "Visitors",
    );
  return tile?.querySelector(":scope > div > div:nth-child(2)")?.textContent ?? "";
}

function renderShell() {
  return render(() => <DashboardShell projectId="1" publicAccess />);
}

function deferred() {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { gate, release };
}

describe("DashboardShell", () => {
  it("opens on the overview for the demo project", async () => {
    const loads = mockShell({ visitors: 1234 });
    renderShell();
    expect(await screen.findByRole("tab", { name: "Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await vi.waitFor(() => expect(visitorsTile()).toBe((1234).toLocaleString()));
    expect(loads[0]?.credentials).toBe("omit");
    expect(screen.queryByRole("tab", { name: "Ask AI" })).toBeNull();
  });

  it("offers Ask AI when chat is available", async () => {
    mockShell({ chat: true });
    renderShell();
    expect(await screen.findByRole("tab", { name: "Ask AI" })).toBeInTheDocument();
  });

  it("asks for a login on a private project without a session", async () => {
    mockApi("get", "/v2/public/chat/status", { available: false });
    render(() => <DashboardShell projectId="42" />);
    expect(await screen.findByText("Log in to view analytics.")).toBeInTheDocument();
  });

  it("switches tab, records it in the URL and loads that tab", async () => {
    mockShell();
    const events = mockApi("get", `${READ}/events`, {
      events: [eventRow("signup")],
      offset: 0,
      limit: 200,
    });
    renderShell();
    await userEvent.click(await screen.findByRole("tab", { name: "Events" }));
    expect(await screen.findByText("signup")).toBeInTheDocument();
    expect(new URLSearchParams(window.location.search).get("tab")).toBe("events");
    expect(events).toHaveLength(1);
  });

  it("opens the tab named in the URL and follows back navigation", async () => {
    mockShell();
    mockApi("get", `${READ}/events`, { events: [eventRow("signup")], offset: 0, limit: 200 });
    window.history.replaceState(null, "", "/dashboard?tab=events");
    renderShell();
    expect(await screen.findByRole("tab", { name: "Events" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    window.history.pushState(null, "", "/dashboard?tab=overview");
    window.dispatchEvent(new PopStateEvent("popstate"));
    await vi.waitFor(() =>
      expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
  });

  it("doesn't retry a failed overview when the URL changes within the tab", async () => {
    mockApi("get", "/v2/public/chat/status", { available: false });
    const loads = mockApi("get", `${READ}/overview`, { error: "boom" }, 500);
    renderShell();
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    await screen.findByRole("tab", { name: "Overview" });
    window.history.pushState(null, "", "/dashboard?tab=overview&x=1");
    window.dispatchEvent(new PopStateEvent("popstate"));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(loads).toHaveLength(1);
  });

  it("maps a legacy landing pages link onto Pages", async () => {
    mockShell();
    mockApi("get", `${READ}/pages`, { pages: [], offset: 0, limit: 200 });
    mockApi("get", `${READ}/landing-pages`, { pages: [], offset: 0, limit: 200 });
    window.history.replaceState(null, "", "/dashboard?tab=landing_pages");
    renderShell();
    expect(await screen.findByRole("tab", { name: "Pages" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(await screen.findByText(/Displaying landing pages for sessions/)).toBeInTheDocument();
  });

  it("fixes the date range on the Revenue tab", async () => {
    mockShell();
    mockApi("get", `${READ}/payment-activity`, { start: 0, end: 0, days: [], monthly_revenue: [] });
    window.history.replaceState(null, "", "/dashboard?tab=revenue");
    renderShell();
    await screen.findByRole("tab", { name: "Revenue" });
    expect(screen.getByRole("button", { name: "Previous period" })).toBeDisabled();
  });

  it("shows the latest range's overview when an earlier request finishes last", async () => {
    const first = deferred();
    let requests = 0;
    mockApi("get", "/v2/public/chat/status", { available: false });
    mockApi("get", `${READ}/overview`, async () => {
      requests += 1;
      if (requests === 1) {
        await first.gate;
        return HttpResponse.json(overview(111));
      }
      return HttpResponse.json(overview(222));
    });
    renderShell();
    await vi.waitFor(() => expect(requests).toBe(1));
    await userEvent.click(await screen.findByRole("button", { name: "Previous period" }));
    await vi.waitFor(() => expect(visitorsTile()).toBe("222"));
    first.release();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(visitorsTile()).toBe("222");
  });

  it("shows the latest range's tab page when an earlier request finishes last", async () => {
    mockShell();
    const first = deferred();
    let requests = 0;
    mockApi("get", `${READ}/events`, async () => {
      requests += 1;
      if (requests === 1) {
        await first.gate;
        return HttpResponse.json({ events: [eventRow("stale-event")], offset: 0, limit: 200 });
      }
      return HttpResponse.json({ events: [eventRow("fresh-event")], offset: 0, limit: 200 });
    });
    window.history.replaceState(null, "", "/dashboard?tab=events");
    renderShell();
    await vi.waitFor(() => expect(requests).toBe(1));
    await userEvent.click(await screen.findByRole("button", { name: "Previous period" }));
    expect(await screen.findByText("fresh-event")).toBeInTheDocument();
    first.release();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByText("stale-event")).toBeNull();
    expect(screen.getByText("fresh-event")).toBeInTheDocument();
  });

  it("keeps a stale range's tab page out of the cache", async () => {
    mockShell();
    const first = deferred();
    let requests = 0;
    mockApi("get", `${READ}/events`, async () => {
      requests += 1;
      if (requests === 1) {
        await first.gate;
        return HttpResponse.json({ events: [eventRow("stale-event")], offset: 0, limit: 200 });
      }
      return HttpResponse.json({ events: [eventRow("fresh-event")], offset: 0, limit: 200 });
    });
    window.history.replaceState(null, "", "/dashboard?tab=events");
    renderShell();
    await vi.waitFor(() => expect(requests).toBe(1));
    await userEvent.click(await screen.findByRole("button", { name: "Previous period" }));
    await screen.findByText("fresh-event");
    first.release();
    await new Promise((resolve) => setTimeout(resolve, 50));

    // Revisiting the tab reads the cache; the old range's page must not be in it.
    await userEvent.click(screen.getByRole("tab", { name: "Overview" }));
    await userEvent.click(screen.getByRole("tab", { name: "Events" }));
    expect(await screen.findByText("fresh-event")).toBeInTheDocument();
    expect(screen.queryByText("stale-event")).toBeNull();
    expect(requests).toBe(2);
  });

  it("shows the rows for the sort on screen when re-sorting overlaps", async () => {
    mockShell();
    const visitorsSort = deferred();
    const conversionsSort = deferred();
    mockApi("get", `${READ}/events`, async ({ url }) => {
      const orderBy = url.searchParams.get("order_by");
      if (orderBy === "visitors") {
        await visitorsSort.gate;
        return HttpResponse.json({ events: [eventRow("by-visitors")], offset: 0, limit: 200 });
      }
      if (orderBy === "conversions") {
        await conversionsSort.gate;
        return HttpResponse.json({ events: [eventRow("by-conversions")], offset: 0, limit: 200 });
      }
      return HttpResponse.json({ events: [eventRow("by-revenue")], offset: 0, limit: 200 });
    });
    window.history.replaceState(null, "", "/dashboard?tab=events");
    renderShell();
    await screen.findByText("by-revenue");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Visitors" }));
    await user.click(screen.getByRole("button", { name: "Conversions" }));
    await user.click(screen.getByRole("button", { name: "Visitors" }));
    conversionsSort.release();
    await new Promise((resolve) => setTimeout(resolve, 20));
    visitorsSort.release();
    expect(await screen.findByText("by-visitors")).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText("by-conversions")).toBeNull();
    expect(screen.getByRole("columnheader", { name: /^Visitors/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
  });

  it("keeps a stale range's page out of the shared cache on the Pages tab", async () => {
    mockShell();
    mockApi("get", `${READ}/landing-pages`, { pages: [], offset: 0, limit: 200 });
    const first = deferred();
    let requests = 0;
    const page = (path: string) => ({
      pages: [{ ...pageRow(), path }],
      offset: 0,
      limit: 200,
    });
    mockApi("get", `${READ}/pages`, async () => {
      requests += 1;
      if (requests === 1) {
        await first.gate;
        return HttpResponse.json(page("/stale"));
      }
      return HttpResponse.json(page("/fresh"));
    });
    window.history.replaceState(null, "", "/dashboard?tab=pages");
    renderShell();
    await vi.waitFor(() => expect(requests).toBeGreaterThan(0));
    await userEvent.click(await screen.findByRole("button", { name: "Previous period" }));
    expect((await screen.findAllByText("x.io/fresh")).length).toBeGreaterThan(0);
    first.release();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByText("x.io/stale")).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "Overview" }));
    await userEvent.click(screen.getByRole("tab", { name: "Pages" }));
    expect((await screen.findAllByText("x.io/fresh")).length).toBeGreaterThan(0);
    expect(screen.queryByText("x.io/stale")).toBeNull();
  });
  it("ignores a cities sort the viewer moved off before it loaded", async () => {
    mockShell();
    const metrics = {
      visitors: 5,
      converting_visitors: 0,
      revenue: 100,
      revenue_per_visitor: 20,
      revenue_change: none,
      visitors_change: none,
      conversions_change: none,
    };
    const audience = (city: string) => ({
      countries: { current: [] },
      cities: { current: [{ city, ...metrics }], offset: 0, limit: 200 },
      browsers: { current: [] },
      os: { current: [] },
      devices: { current: [] },
      screen_sizes: { current: [] },
      viewports: { current: [] },
    });
    const byName = deferred();
    mockApi("get", `${READ}/audience`, async ({ url }) => {
      if (url.searchParams.get("order_by") === "name") {
        await byName.gate;
        return HttpResponse.json(audience("SortedByNameCity"));
      }
      return HttpResponse.json(audience("SortedByRevenueCity"));
    });
    window.history.replaceState(null, "", "/dashboard?tab=audience");
    renderShell();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Cities" }));
    const citiesTable = () =>
      screen.getByRole("columnheader", { name: /^City/ }).closest("table") as HTMLElement;
    await vi.waitFor(() =>
      expect(within(citiesTable()).getByText("SortedByRevenueCity")).toBeInTheDocument(),
    );
    await user.click(within(citiesTable()).getByRole("button", { name: "City" }));
    await user.click(within(citiesTable()).getByRole("button", { name: /^Revenue/ }));
    byName.release();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(within(citiesTable()).queryByText("SortedByNameCity")).toBeNull();
    expect(within(citiesTable()).getByText("SortedByRevenueCity")).toBeInTheDocument();
    expect(within(citiesTable()).getByRole("columnheader", { name: /^Revenue/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
  });
});

function pageRow() {
  return {
    id: 3,
    path: "/",
    hostname: "x.io",
    visitors: 10,
    converting_visitors: 0,
    attributed_revenue: 1000,
    conversion_rate: 0,
    conversion_lower_bound: 0,
    bounce_rate: 0,
    exit_rate: 0,
    avg_session_duration: 0,
    avg_page_duration: 0,
    avg_scroll_percent: 0,
    revenue_per_visitor: 0,
    increase_visitors_25pct: 0,
    increase_conversion_25pct: 0,
    reduce_exits_25pct: 0,
    reduce_bounces_25pct: 0,
    increase_session_25pct: 0,
    best_visitors_uplift: false,
    best_conversion_uplift: false,
    best_exit_uplift: false,
    best_bounce_uplift: false,
    best_session_uplift: false,
    highest_exit_rate: false,
    highest_bounce_rate: false,
  };
}

describe("DashboardShell projects tab", () => {
  afterEach(async () => {
    await signOut();
  });

  function renderSignedIn(plan: string, projects: Record<string, ReturnType<typeof testProject>>) {
    const session = signIn(testSession({ plan, projects }));
    mockApi("get", "/v2/public/chat/status", { available: false });
    mockApi("get", "/v2/read/projects/:id/imported-coverage", {
      include_in_reports: true,
      has_data: false,
      has_events: false,
      ranges: [],
    });
    for (const project of Object.values(projects)) {
      mockApi("get", `/v2/read/projects/${project.id}/overview`, overview(3));
    }
    const onProjectChange = vi.fn();
    const first = Object.values(projects)[0]?.id ?? "";
    render(() => (
      <DashboardShell
        projectId={first}
        projects={projectOptions(session.projects)}
        onProjectChange={onProjectChange}
      />
    ));
    return { onProjectChange };
  }

  it("lists every project and opens one on its Overview", async () => {
    window.history.replaceState(null, "", "/?tab=projects");
    const { onProjectChange } = renderSignedIn("growth", {
      alpha_com: testProject("7"),
      beta_com: testProject("9", "Asia/Tokyo"),
    });
    expect(await screen.findByRole("tab", { name: "Projects" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await userEvent.setup().click(await screen.findByRole("button", { name: /^beta_com/ }));
    // Opens on the period the card showed, on beta's own (Tokyo) days.
    expect(onProjectChange).toHaveBeenCalledWith("9", expect.any(Object));
    const carried = onProjectChange.mock.calls[0]?.[1] as { start: number; end: number };
    expect(new Date(carried.start * 1000).getUTCHours()).toBe(15);
    expect(new URLSearchParams(window.location.search).get("tab")).toBe("overview");
  });

  it("is absent on a single-website plan with one project", async () => {
    window.history.replaceState(null, "", "/?tab=projects");
    renderSignedIn("starter", { alpha_com: testProject("7") });
    expect(await screen.findByRole("tab", { name: "Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.queryByRole("tab", { name: "Projects" })).toBeNull();
  });

  it("is absent on the public demo", async () => {
    window.history.replaceState(null, "", "/?tab=projects");
    mockShell();
    renderShell();
    expect(await screen.findByRole("tab", { name: "Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.queryByRole("tab", { name: "Projects" })).toBeNull();
  });
});

describe("DashboardShell imported data notice", () => {
  const NOTICE = "Imported Google Analytics data isn't included in this view.";

  afterEach(async () => {
    await signOut();
  });

  const renderSignedIn = () => {
    signIn(testSession({ projects: { acme_test: testProject("1", "UTC") } }));
    return render(() => <DashboardShell projectId="1" />);
  };

  it("stays away from a view that counts the imported data", async () => {
    const loads = mockShell({ coverageHasData: true });
    renderSignedIn();
    await vi.waitFor(() => expect(loads.length).toBeGreaterThan(0));
    await screen.findByRole("tab", { name: "Overview" });
    expect(screen.queryByText(NOTICE)).toBeNull();
  });

  it("appears on a tab that can't use imported data, once there's some in range", async () => {
    mockShell({ coverageHasData: true });
    mockApi("get", `${READ}/funnels`, { funnels: [] });
    mockApi("get", `${READ}/funnel-data`, { funnels: [] });
    renderSignedIn();
    await userEvent.click(await screen.findByRole("tab", { name: "Funnels" }));
    expect(await screen.findByText(NOTICE)).toBeTruthy();
  });

  it("stays away when the range has no imported data", async () => {
    mockShell({ coverageHasData: false });
    mockApi("get", `${READ}/funnels`, { funnels: [] });
    mockApi("get", `${READ}/funnel-data`, { funnels: [] });
    renderSignedIn();
    await userEvent.click(await screen.findByRole("tab", { name: "Funnels" }));
    await vi.waitFor(() => expect(screen.getByRole("tab", { name: "Funnels" })).toBeTruthy());
    expect(screen.queryByText(NOTICE)).toBeNull();
  });
});
