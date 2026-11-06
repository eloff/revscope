import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { PaymentLanding } from "../../lib/paymentsApi";
import ActiveFilters from "./ActiveFilters";
import MetricTile from "./MetricTile";
import NoDataView from "./NoDataView";
import SourceLinkCell from "./SourceLinkCell";
import TabDocsLink from "./TabDocsLink";
import ViewDetailsButton from "./ViewDetailsButton";

describe("ActiveFilters", () => {
  it("renders nothing without filters", () => {
    const { container } = render(() => <ActiveFilters filters={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("removes the filter whose chip was dismissed", async () => {
    const removeUs = vi.fn();
    const removePricing = vi.fn();
    render(() => (
      <ActiveFilters
        filters={[
          { label: "Country: US", onRemove: removeUs },
          { label: "Page: /pricing", onRemove: removePricing },
        ]}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "Remove filter: Page: /pricing" }));
    expect(removePricing).toHaveBeenCalledOnce();
    expect(removeUs).not.toHaveBeenCalled();
  });
});

describe("MetricTile", () => {
  it.each([
    [0.256, true, "▲26%"],
    [0.0123, true, "▲1.2%"],
    [-0.05, true, "▼5%"],
    [0.0995, true, "▲10%"],
    [0.0004, true, "0%"],
    [0, true, "0%"],
  ])("shows a change of %d as %s", (ratio, higherIsBetter, text) => {
    render(() => (
      <MetricTile
        label="Visitors"
        value="1,234"
        change={{ ratio }}
        higherIsBetter={higherIsBetter}
      />
    ));
    expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("colors a change that rounds to 0% as flat", () => {
    render(() => (
      <>
        <MetricTile label="Flat" value="1" change={{ ratio: 0 }} />
        <MetricTile label="Up" value="1" change={{ ratio: 0.0004 }} />
        <MetricTile label="Down" value="1" change={{ ratio: -0.0004 }} />
      </>
    ));
    const [flat, up, down] = screen.getAllByText("0%");
    expect(up?.className).toBe(flat?.className);
    expect(down?.className).toBe(flat?.className);
  });

  it("colors a drop as good when lower is better", () => {
    render(() => (
      <>
        <MetricTile label="Bounce" value="40%" change={{ ratio: -0.1 }} higherIsBetter={false} />
        <MetricTile label="Visitors" value="9" change={{ ratio: -0.1 }} />
      </>
    ));
    const [good, bad] = screen.getAllByText("▼10%");
    expect(good?.className).not.toBe(bad?.className);
  });

  it("hides the change without a comparison and while loading", () => {
    render(() => (
      <>
        <MetricTile label="A" value="1" change={{ ratio: null }} />
        <MetricTile label="B" value="2" change={{ ratio: Number.POSITIVE_INFINITY }} />
        <MetricTile label="C" value="3" change={{ ratio: 0.5 }} loading />
      </>
    ));
    expect(screen.queryByText(/[▲▼→]/)).toBeNull();
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.queryByText("3")).toBeNull();
  });

  it("borders an active tile in its series color", () => {
    const { container } = render(() => (
      <MetricTile label="MRR" value="$1" unit="/mo" active accentColor="rgb(1, 2, 3)" />
    ));
    expect(container.firstElementChild).toHaveStyle({ "border-color": "rgb(1, 2, 3)" });
    expect(screen.getByText("/mo")).toBeInTheDocument();
  });
});

describe("NoDataView", () => {
  it("defaults to the date-range copy with setup links", () => {
    render(() => <NoDataView />);
    expect(screen.getByRole("heading")).toHaveTextContent("No data for the selected date range");
    expect(screen.getByRole("link", { name: "Getting started" })).toHaveAttribute(
      "href",
      "/docs/getting-started",
    );
    expect(screen.getByRole("link", { name: "Payment integrations" })).toBeInTheDocument();
  });

  it("points crawlers at the Cloudflare integration only", () => {
    render(() => <NoDataView variant="crawlers" />);
    expect(screen.getByRole("link", { name: "Cloudflare integration" })).toHaveAttribute(
      "href",
      "/settings",
    );
    expect(screen.getAllByRole("link")).toHaveLength(1);
  });
});

describe("TabDocsLink and ViewDetailsButton", () => {
  it("links docs with an extra class", () => {
    render(() => <TabDocsLink href="/docs/pages" label="Pages docs" class="extra" />);
    const link = screen.getByRole("link", { name: "Pages docs" });
    expect(link).toHaveAttribute("href", "/docs/pages");
    expect(link.className).toContain("extra");
  });

  it("names the details button by its title", async () => {
    const onClick = vi.fn();
    render(() => (
      <ViewDetailsButton title="View details for /a" onClick={onClick}>
        /a
      </ViewDetailsButton>
    ));
    await userEvent.click(screen.getByRole("button", { name: "View details for /a" }));
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("SourceLinkCell", () => {
  const referrer = {
    referrer: { id: 4, referrer: "news.ycombinator.com" },
    referrer_type: undefined,
  } as unknown as PaymentLanding;

  it("shows a dash without a landing", () => {
    const { container } = render(() => <SourceLinkCell landing={undefined} />);
    expect(container).toHaveTextContent("—");
  });

  it("shows the group and plain detail when it can't link", () => {
    render(() => <SourceLinkCell landing={referrer} />);
    expect(screen.getByText("Referrers")).toBeInTheDocument();
    expect(screen.getByText("news.ycombinator.com")).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("links to the source without triggering the row", async () => {
    const onSelect = vi.fn();
    const onRow = vi.fn();
    render(() => (
      <table>
        <tbody>
          <tr onClick={onRow}>
            <td>
              <SourceLinkCell landing={referrer} onRevenueSourceSelect={onSelect} />
            </td>
          </tr>
        </tbody>
      </table>
    ));
    await userEvent.click(
      screen.getByRole("button", { name: "View details for news.ycombinator.com in Sources" }),
    );
    expect(onSelect).toHaveBeenCalledWith({ kind: "referrer", id: 4 });
    expect(onRow).not.toHaveBeenCalled();
  });

  it("labels direct traffic", () => {
    render(() => <SourceLinkCell landing={{} as PaymentLanding} onRevenueSourceSelect={vi.fn()} />);
    expect(
      screen.getByRole("button", { name: "View details for Direct in Sources" }),
    ).toBeInTheDocument();
  });
});
