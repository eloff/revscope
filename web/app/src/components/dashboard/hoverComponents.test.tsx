import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import type { JSX } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import CountryLinkCell, { visitorLocationLabel } from "./CountryLinkCell";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import GridColumnInfoIcon from "./GridColumnInfoIcon";
import HoverHighlightDescriptions from "./HoverHighlightDescriptions";
import MetricHoverCard from "./MetricHoverCard";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { hoverCardStatValueNeutral } from "./MetricHoverCardContent.css";
import type { MetricHoverContent } from "./metricHoverDetails";

function withDashboard(children: () => JSX.Element) {
  return render(() => <DashboardStateProvider>{children()}</DashboardStateProvider>);
}

describe("CountryLinkCell", () => {
  it("names a country from its code with a flag", () => {
    const { container } = render(() => <CountryLinkCell country="de" />);
    expect(screen.getByText("Germany")).toBeInTheDocument();
    expect(container.querySelector(".flag.de")).not.toBeNull();
  });

  it("links to Audience without triggering the row", async () => {
    const onCountrySelect = vi.fn();
    const onRow = vi.fn();
    render(() => (
      <table>
        <tbody>
          <tr onClick={onRow}>
            <td>
              <CountryLinkCell
                country="fr"
                label="Paris, France"
                onCountrySelect={onCountrySelect}
              />
            </td>
          </tr>
        </tbody>
      </table>
    ));
    await userEvent.click(
      screen.getByRole("button", { name: "View details for France in Audience" }),
    );
    expect(onCountrySelect).toHaveBeenCalledWith("FR");
    expect(onRow).not.toHaveBeenCalled();
    expect(screen.getByText("Paris, France")).toBeInTheDocument();
  });

  it("shows unknown values as text and a missing one as a dash", () => {
    render(() => (
      <>
        <span>
          <CountryLinkCell country="Atlantis" onCountrySelect={vi.fn()} />
        </span>
        <span>
          <CountryLinkCell />
        </span>
      </>
    ));
    expect(screen.getByText("Atlantis")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("builds a location label", () => {
    expect(visitorLocationLabel("jp", "Osaka")).toBe("Osaka, Japan");
    expect(visitorLocationLabel("Narnia")).toBe("Narnia");
    expect(visitorLocationLabel(undefined, "Osaka")).toBe("Osaka");
    expect(visitorLocationLabel()).toBeUndefined();
  });
});

describe("GridColumnInfoIcon", () => {
  it("explains the column on hover", async () => {
    render(() => (
      <GridColumnInfoIcon title="Exit rate" description="Share of sessions that ended here." />
    ));
    await userEvent.hover(screen.getByRole("button", { name: "About Exit rate" }));
    expect(await screen.findByText("Share of sessions that ended here.")).toBeInTheDocument();
  });

  it("labels a warning as one", () => {
    render(() => <GridColumnInfoIcon title="Revenue" description="x" tone="warning" />);
    expect(screen.getByRole("button", { name: "Warning: Revenue" })).toBeInTheDocument();
  });
});

describe("HoverHighlightDescriptions", () => {
  it("renders metric labels, custom lines and source group badges", () => {
    const { container } = render(() => (
      <HoverHighlightDescriptions
        lines={[
          { metric: "revenue", sourceGroup: "search" },
          { metric: "highestExitRate" },
          { description: "Custom note", iconClass: "my-icon" },
        ]}
      />
    ));
    expect(screen.getByText("Best revenue", { exact: false })).toHaveTextContent(
      "Best revenue in Search",
    );
    expect(screen.getByText("Highest exit rate. Focus here.")).toBeInTheDocument();
    expect(screen.getByText("Custom note")).toBeInTheDocument();
    expect(container.querySelector(".my-icon")).not.toBeNull();
    expect(screen.getAllByText("★")).toHaveLength(1);
  });
});

describe("MetricHoverCardContent", () => {
  const content: MetricHoverContent = {
    stats: [
      { label: "Revenue", value: "$9", changeRatio: 0.25 },
      { label: "Refunds", value: "$1", changeRatio: 0.5, higherIsBetter: false },
      { label: "Growth", value: "", changeRatio: 0.1, changeInValueColumn: true },
      { label: "Flat", value: "3", changeRatio: 0 },
    ],
    landingPaths: [{ id: 5, path: "/pricing", visitors: 1200, share: 0.4 }],
    description: "What this means.",
    highlightDescriptions: [{ metric: "conversion" }],
  };

  it("shows a flat value-column change as 0% and a missing one as a dash", () => {
    const rates: MetricHoverContent = {
      stats: [
        { label: "Zero", value: "", changeRatio: 0, changeInValueColumn: true },
        { label: "Tiny", value: "", changeRatio: 0.0004, changeInValueColumn: true },
        { label: "Up", value: "", changeRatio: 0.05, changeInValueColumn: true },
        { label: "Missing", value: "—" },
      ],
    };
    withDashboard(() => <MetricHoverCardContent title="MRR" content={rates} />);
    const value = (label: string) => screen.getByText(label).nextElementSibling?.textContent;
    expect(value("Zero")).toBe("0%");
    expect(value("Tiny")).toBe("0%");
    expect(value("Up")).toBe("▲5%");
    expect(value("Missing")).toBe("—");
    const toneClass = (label: string) =>
      screen.getByText(label).nextElementSibling?.firstElementChild?.className;
    expect(toneClass("Zero")).toBe(hoverCardStatValueNeutral);
    expect(toneClass("Tiny")).toBe(hoverCardStatValueNeutral);
  });

  it("shows stats with their change, landing paths and notes", () => {
    withDashboard(() => <MetricHoverCardContent title="Total revenue" content={content} />);
    expect(screen.getByText("Total revenue")).toBeInTheDocument();
    const revenue = screen.getByText("Revenue").nextElementSibling;
    expect(revenue).toHaveTextContent("$9");
    expect(revenue?.textContent).toMatch(/25/);
    expect(screen.getByText("Flat").nextElementSibling).toHaveTextContent(/^3$/);
    expect(screen.getByText("/pricing")).toBeInTheDocument();
    expect(screen.getByText(`${(1200).toLocaleString()} (40.0%)`)).toBeInTheDocument();
    expect(screen.getByText("What this means.")).toBeInTheDocument();
    expect(screen.getByText("Best conversion rate")).toBeInTheDocument();
    expect(screen.getByText(/vs the prior 30 days/)).toBeInTheDocument();
  });

  it("colors a rising cost as bad", () => {
    withDashboard(() => <MetricHoverCardContent title="t" content={content} />);
    const good = screen.getByText("Revenue").nextElementSibling?.querySelector("span");
    const bad = screen.getByText("Refunds").nextElementSibling?.querySelector("span");
    expect(good?.className).not.toBe(bad?.className);
  });

  it("links landing paths when asked and can drop the comparison note", async () => {
    const onLandingPathSelect = vi.fn();
    render(() => (
      <MetricHoverCardContent
        title="t"
        content={content}
        statLayout="threeColumn"
        onLandingPathSelect={onLandingPathSelect}
        showPeriodComparisonNote={false}
      />
    ));
    await userEvent.click(
      screen.getByRole("button", { name: "View details for /pricing in Landing Pages" }),
    );
    expect(onLandingPathSelect).toHaveBeenCalledWith(5);
    expect(screen.queryByText(/prior 30 days/)).toBeNull();
  });

  it("shows loading and empty states", () => {
    render(() => (
      <>
        <MetricHoverCardContent title="a" content={content} loading />
        <MetricHoverCardContent title="b" content={undefined} />
      </>
    ));
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(screen.getByText("No data for this period.")).toBeInTheDocument();
  });
});

describe("MetricHoverCard", () => {
  it("opens its content from the rendered trigger", async () => {
    withDashboard(() => (
      <MetricHoverCard
        title="Visitors"
        content={{ stats: [{ label: "Bounced sessions", value: "4" }] }}
        renderTrigger={(merge) => (
          <button type="button" {...merge({})}>
            Visitors tile
          </button>
        )}
      />
    ));
    await userEvent.hover(screen.getByRole("button", { name: "Visitors tile" }));
    expect(await screen.findByText("Bounced sessions")).toBeInTheDocument();
  });
});
