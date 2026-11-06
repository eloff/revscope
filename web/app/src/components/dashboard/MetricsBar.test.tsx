import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { lastChart } from "../../../test/apexchartsMock";
import { testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import { chartVisibility, setChartVisibility } from "../../lib/dashboardPreferences";
import { createDefaultChartVisibility } from "./chartMetrics";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import MetricsBar from "./MetricsBar";
import Sparkline from "./Sparkline";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => setChartVisibility(createDefaultChartVisibility()));

const current = testPeriodMetrics({
  total_revenue: 1_500_000,
  visitors: 1234,
  refunds_and_chargebacks: -20_000,
  churn_mrr: 5000,
  conversion_rate: 0.01234,
  bounce_rate: 0.4,
});

function renderBar(overrides: Parameters<typeof testOverview>[0] = {}) {
  const overview = testOverview(overrides);
  return render(() => (
    <DashboardStateProvider>
      <MetricsBar data={{ current, previous: testPeriodMetrics() }} overview={overview} />
    </DashboardStateProvider>
  ));
}

/** The toggle wrapping the tile whose label reads `label` (hover cards repeat the label). */
function findTile(label: string): HTMLElement | undefined {
  return screen
    .queryAllByRole("button")
    .find(
      (button) => button.querySelector(":scope > div > div:first-child")?.textContent === label,
    );
}

function tile(label: string): HTMLElement {
  const element = findTile(label);
  if (!element) throw new Error(`no tile ${label}`);
  return element;
}

describe("MetricsBar", () => {
  it("formats each tile and hides empty optional ones", () => {
    renderBar();
    expect(tile("Revenue")).toHaveTextContent("$1.5k");
    expect(tile("Visitors")).toHaveTextContent((1234).toLocaleString());
    expect(tile("Refunds")).toHaveTextContent("$20");
    expect(tile("Conversion rate")).toHaveTextContent("1.234%");
    expect(findTile("MRR")).toBeUndefined();
    expect(findTile("Session time")).toBeUndefined();
  });

  it("shows every tile, blank, while loading", () => {
    render(() => (
      <DashboardStateProvider>
        <MetricsBar data={undefined} overview={undefined} loading />
      </DashboardStateProvider>
    ));
    expect(screen.getAllByText("—")).toHaveLength(11);
  });

  it("renders nothing without data", () => {
    const { container } = render(() => <MetricsBar data={undefined} overview={undefined} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("toggles the tile's series on the chart", async () => {
    renderBar();
    const bounce = tile("Bounce rate");
    expect(bounce).toHaveAttribute("title", "Show Bounce rate on chart");
    await userEvent.click(bounce);
    expect(chartVisibility().lineMetric).toBe("bounce_rate");
    expect(tile("Bounce rate")).toHaveAttribute("title", "Hide Bounce rate from chart");
    expect(tile("Visitors")).toHaveAttribute("aria-pressed", "false");
  });

  it("shows only the series picked while revenue is hidden", async () => {
    render(() => (
      <DashboardStateProvider>
        <MetricsBar
          data={{
            current: { ...current, mrr: 9000, new_revenue: 4000 },
            previous: testPeriodMetrics(),
          }}
          overview={testOverview()}
        />
      </DashboardStateProvider>
    ));
    const user = userEvent.setup();
    await user.click(tile("Revenue"));
    expect(tile("MRR")).toHaveAttribute("aria-pressed", "false");

    await user.click(tile("MRR"));
    expect(tile("MRR")).toHaveAttribute("aria-pressed", "true");
    expect(tile("Revenue")).toHaveAttribute("aria-pressed", "true");
    expect(tile("New revenue")).toHaveAttribute("aria-pressed", "false");

    await user.click(tile("MRR"));
    expect(tile("MRR")).toHaveAttribute("aria-pressed", "false");
  });

  it("treats rising costs as bad news", () => {
    const changes = { ...testOverview().changes };
    changes.total_revenue = { ratio: 0.5 };
    changes.churn_mrr = { ratio: 0.5 };
    changes.refunds_and_chargebacks = { ratio: 0.5 };
    renderBar({ changes });
    const tone = (label: string) => tile(label).querySelector(":scope > div > span")?.className;
    expect(tone("Churned MRR")).not.toBe(tone("Revenue"));
    expect(tone("Refunds")).toBe(tone("Churned MRR"));
  });
});

describe("Sparkline", () => {
  it("plots the values with numbered categories by default", () => {
    render(() => <Sparkline values={[1, 4, 2]} color="#abcdef" />);
    const options = lastChart().options;
    expect(options.series).toEqual([{ name: "Views", data: [1, 4, 2] }]);
    expect(options.xaxis?.categories).toEqual(["1", "2", "3"]);
    expect(options.colors).toEqual(["#abcdef"]);
    const formatter = options.tooltip?.y as { formatter: (value: number) => string };
    expect(formatter.formatter(1234.4)).toBe(`${(1234).toLocaleString()} views`);
  });
});
