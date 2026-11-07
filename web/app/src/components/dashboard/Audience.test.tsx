import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { charts } from "../../../test/apexchartsMock";
import type { AudienceResponse, RowMetrics } from "../../lib/audienceApi";
import { audienceBreakdownSorts, setAudienceBreakdownSorts } from "../../lib/dashboardPreferences";
import Audience, { type AudienceProps } from "./Audience";
import { DEFAULT_AUDIENCE_BREAKDOWN_SORT } from "./audienceLogic";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));
vi.mock("jsvectormap", () => ({
  default: class {
    destroy() {}
  },
}));
vi.mock("jsvectormap/dist/maps/world.js", () => ({}));

const defaultSorts = audienceBreakdownSorts();
afterEach(() => setAudienceBreakdownSorts(defaultSorts));

function metrics(visitors: number, revenue: number, converting = 1): RowMetrics {
  return {
    visitors,
    converting_visitors: converting,
    revenue,
    revenue_per_visitor: revenue / visitors,
    revenue_change: { ratio: null },
    visitors_change: { ratio: null },
    conversions_change: { ratio: null },
  } as RowMetrics;
}

const country = (
  code: string,
  name: string,
  visitors: number,
  revenue: number,
  lowerBound: number,
) => ({
  country: code,
  country_name: name,
  conversion_rate: lowerBound * 2,
  conversion_lower_bound: lowerBound,
  conversion_ci: { lower: lowerBound, upper: lowerBound * 3 },
  ...metrics(visitors, revenue),
});

const data: AudienceResponse = {
  countries: {
    current: [
      country("US", "United States", 500, 90_000, 0.01),
      country("DE", "Germany", 100, 30_000, 0.05),
    ],
  },
  cities: {
    current: [{ city: "Berlin", country: "DE", ...metrics(80, 20_000) }],
    offset: 0,
    limit: 200,
  },
  browsers: {
    current: [
      { browser_family: "firefox", browser_family_name: "Firefox", ...metrics(70, 10_000) },
    ],
  },
  os: { current: [{ os: "linux", os_name: "Linux", ...metrics(60, 5_000) }] },
  devices: { current: [] },
  screen_sizes: { current: [] },
  viewports: { current: [] },
} as unknown as AudienceResponse;

function renderAudience(overrides: Partial<AudienceProps> = {}) {
  const props: AudienceProps = {
    data,
    treemapData: data,
    loading: false,
    treemapLoading: false,
    error: "",
    ...overrides,
  };
  let state!: ReturnType<typeof useDashboardState>;
  const Capture = () => {
    state = useDashboardState();
    return null;
  };
  render(() => (
    <DashboardStateProvider>
      <Capture />
      <Audience {...props} />
    </DashboardStateProvider>
  ));
  return { props, state };
}

const countryTable = () =>
  screen.getByRole("columnheader", { name: /^Country/ }).closest("table") as HTMLElement;
const countryNames = () =>
  within(countryTable())
    .getAllByRole("row")
    .slice(1)
    .map((row) => row.querySelector("td")?.textContent);

describe("Audience", () => {
  it("ranks countries by revenue, starring the best revenue and conversion", () => {
    renderAudience();
    expect(countryNames()).toEqual(["United States", "Germany"]);
    expect(screen.getByText("2 countries")).toBeInTheDocument();
    const [us, de] = within(countryTable()).getAllByRole("row").slice(1);
    expect(us?.querySelector('[class*="gridBest"]')).not.toBeNull();
    expect(de?.querySelector('[class*="gridBest"]')).not.toBeNull();
    expect(screen.getByText("Revenue share")).toBeInTheDocument();
  });

  it("stars nothing and keys the map to visitors when nothing converted", () => {
    const unconverted = {
      ...data,
      countries: {
        current: [
          { ...country("US", "United States", 500, 0, 0), ...metrics(500, 0, 0) },
          { ...country("DE", "Germany", 100, 0, 0), ...metrics(100, 0, 0) },
        ],
      },
    } as AudienceResponse;
    renderAudience({ data: unconverted, treemapData: unconverted });
    expect(countryTable().querySelector('[class*="gridBest"]')).toBeNull();
    const legend = screen.getByText("Less").closest("div")?.parentElement as HTMLElement;
    expect(legend.firstElementChild?.textContent).toBe("Visitors");
  });

  it("sorts countries, remembering the choice", async () => {
    renderAudience();
    await userEvent.click(within(countryTable()).getByRole("button", { name: "Country" }));
    expect(audienceBreakdownSorts().country).toEqual({ column: "name", direction: "desc" });
    expect(countryNames()).toEqual(["United States", "Germany"]);
    await userEvent.click(within(countryTable()).getByRole("button", { name: /Country/ }));
    expect(countryNames()).toEqual(["Germany", "United States"]);
    expect(audienceBreakdownSorts().browser).toEqual(DEFAULT_AUDIENCE_BREAKDOWN_SORT);
  });

  it("filters the dashboard by a country", async () => {
    const onCountryFilter = vi.fn();
    renderAudience({ onCountryFilter });
    await userEvent.click(screen.getByRole("button", { name: "Filter by Germany" }));
    expect(onCountryFilter).toHaveBeenCalledWith("DE", "Germany");
  });

  it("marks a country already in the segment filters", () => {
    const { state } = renderAudience({ onCountryFilter: vi.fn() });
    state.addCountryFilter("DE", "Germany");
    expect(screen.getByRole("button", { name: "Filtering by Germany" })).toBeInTheDocument();
  });

  it("filters the dashboard by a browser or OS and marks active ones", async () => {
    const onAttributeFilter = vi.fn();
    const { state } = renderAudience({ onAttributeFilter });
    await userEvent.click(screen.getByRole("button", { name: "Filter by Firefox" }));
    expect(onAttributeFilter).toHaveBeenCalledWith({
      attribute: "browser",
      value: "firefox",
      name: "Firefox",
    });
    await userEvent.click(screen.getByRole("tab", { name: "OS" }));
    state.addAttributeFilter({ attribute: "os", value: "linux", name: "Linux" });
    expect(await screen.findByRole("button", { name: "Filtering by Linux" })).toBeInTheDocument();
    const screenSize = screen.getByRole("tab", { name: "Screen size" });
    await userEvent.click(screenSize);
    const panel = document.getElementById(screenSize.getAttribute("aria-controls") ?? "");
    expect(panel).not.toBeNull();
    expect(within(panel as HTMLElement).queryByRole("columnheader", { name: "Filter" })).toBeNull();
  });

  it("shows a country's hover card from its row", async () => {
    renderAudience();
    const [, germany] = within(countryTable()).getAllByRole("row").slice(1);
    fireEvent.mouseMove(germany as HTMLElement, { clientX: 5, clientY: 5 });
    expect(await screen.findByText("Best conversion rate")).toBeInTheDocument();
  });

  it("switches the breakdown to cities and the tech tabs", async () => {
    renderAudience();
    await userEvent.click(screen.getByRole("tab", { name: "Cities" }));
    expect(await screen.findByText("Berlin, Germany")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "OS" }));
    expect(await screen.findByText("Linux")).toBeInTheDocument();
    expect(screen.getByText("Firefox")).toBeInTheDocument();
  });

  it("selects a country from its treemap tile", () => {
    const onCountrySelect = vi.fn();
    renderAudience({ onCountrySelect });
    const geoTreemap = charts.find((chart) =>
      JSON.stringify(chart.options.series ?? []).includes('"US"'),
    );
    geoTreemap?.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(onCountrySelect).toHaveBeenCalledWith("US");
  });

  it("pages cities from the server", async () => {
    const onCitiesPageChange = vi.fn();
    renderAudience({
      onCitiesPageChange,
      data: { ...data, cities: { ...data.cities, next_offset: 200 } },
    });
    await userEvent.click(screen.getByRole("tab", { name: "Cities" }));
    const citiesTable = screen.getByRole("columnheader", { name: /^City/ }).closest("table")
      ?.parentElement as HTMLElement;
    await userEvent.click(within(citiesTable).getByRole("button", { name: "Next" }));
    expect(onCitiesPageChange).toHaveBeenCalledWith(200);
  });

  it("stars the best city only on a revenue-descending sort", async () => {
    renderAudience();
    await userEvent.click(screen.getByRole("tab", { name: "Cities" }));
    const berlin = (await screen.findByText("Berlin, Germany")).closest("tr") as HTMLElement;
    expect(berlin.querySelector('[class*="gridBest"]')).not.toBeNull();
    setAudienceBreakdownSorts({
      ...defaultSorts,
      cities: { column: "visitors", direction: "desc" },
    });
    expect(berlin.querySelector('[class*="gridBest"]')).toBeNull();
  });

  it("stars no city while the page on show is left over from another sort", async () => {
    renderAudience({ citiesStale: true });
    await userEvent.click(screen.getByRole("tab", { name: "Cities" }));
    const berlin = (await screen.findByText("Berlin, Germany")).closest("tr") as HTMLElement;
    expect(berlin.querySelector('[class*="gridBest"]')).toBeNull();
  });

  it("keeps the revenue legend while the countries load", () => {
    renderAudience({ data: undefined, treemapData: undefined, loading: true });
    expect(screen.getByText("Revenue share")).toBeInTheDocument();
  });

  it("shows the tab error on the map", () => {
    renderAudience({ error: "Audience failed." });
    expect(screen.getByText("Audience failed.")).toBeInTheDocument();
  });
});
