import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import type { JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { testLanding, testPayment, testVisitor } from "../../../test/dashboardFixtures";
import { conversionsSort, setConversionsSort } from "../../lib/dashboardPreferences";
import { DEFAULT_VISITOR_SORT, type VisitorSortState } from "../../lib/visitorsApi";
import { DEFAULT_CONVERSIONS_SORT } from "./conversionsLogic";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import PaymentsTable from "./PaymentsTable";
import VisitorsTable from "./VisitorsTable";

afterEach(() => setConversionsSort(DEFAULT_CONVERSIONS_SORT));

function withDashboard(children: () => JSX.Element) {
  return render(() => <DashboardStateProvider>{children()}</DashboardStateProvider>);
}

const bodyRows = () =>
  within(screen.getAllByRole("rowgroup")[1] as HTMLElement).queryAllByRole("row");

describe("PaymentsTable", () => {
  const payments = [
    testPayment({ id: 1, customer_email: "zed@x.io", amount: 10_000, paid_at: 100 }),
    testPayment({
      id: 2,
      customer_email: "amy@x.io",
      amount: 90_000,
      paid_at: 200,
      interval_unit: "month",
      landing: testLanding({ country: "DE", viewed_at: 200 - 86_400 }),
    }),
  ];

  it("lists payments newest first with customer, amount and time to purchase", () => {
    withDashboard(() => <PaymentsTable payments={payments} loading={false} emptyLabel="None" />);
    const rows = bodyRows();
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("amy@x.io");
    expect(rows[0]).toHaveTextContent("$90");
    expect(rows[0]).toHaveTextContent("Monthly");
    expect(rows[0]).toHaveTextContent("1 day");
    expect(rows[0]).toHaveTextContent("Germany");
    expect(rows[1]).toHaveTextContent("Single Payment");
    expect(screen.getByText("2 conversions")).toBeInTheDocument();
  });

  it("sorts by a column, remembering the choice", async () => {
    withDashboard(() => <PaymentsTable payments={payments} loading={false} emptyLabel="None" />);
    await userEvent.click(screen.getByRole("button", { name: "Customer" }));
    expect(conversionsSort()).toEqual({ column: "customer", direction: "desc" });
    expect(bodyRows()[0]).toHaveTextContent("zed@x.io");
    expect(screen.getByRole("columnheader", { name: /Customer/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
    await userEvent.click(screen.getByRole("button", { name: /Customer/ }));
    expect(bodyRows()[0]).toHaveTextContent("amy@x.io");
  });

  it("opens a payment by click or keyboard", async () => {
    const onPaymentSelect = vi.fn();
    withDashboard(() => (
      <PaymentsTable
        payments={payments}
        loading={false}
        emptyLabel="None"
        onPaymentSelect={onPaymentSelect}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    screen.getByRole("button", { name: "View journey for zed@x.io" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onPaymentSelect.mock.calls.map(([payment]) => payment.id)).toEqual([2, 1]);
  });

  it("pages through results", async () => {
    const onPageChange = vi.fn();
    withDashboard(() => (
      <PaymentsTable
        payments={payments}
        loading={false}
        emptyLabel="None"
        offset={100}
        hasNextPage={false}
        onPageChange={onPageChange}
      />
    ));
    expect(screen.getByText("Showing 101-102 conversions")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Previous" }));
    expect(onPageChange).toHaveBeenCalledWith("previous");
  });

  it("shows loading before the first page and the empty label after", () => {
    withDashboard(() => (
      <>
        <PaymentsTable payments={undefined} loading emptyLabel="None" loadingLabel="Fetching…" />
        <PaymentsTable payments={[]} loading={false} emptyLabel="No conversions yet." />
      </>
    ));
    expect(screen.getByText("Fetching…")).toBeInTheDocument();
    expect(screen.getByText("No conversions yet.")).toBeInTheDocument();
  });
});

describe("VisitorsTable", () => {
  const visitors = [
    testVisitor({ visitor_timestamp_id: "aaa", country: "jp", city: "Osaka" }),
    testVisitor({ visitor_timestamp_id: "bbb" }),
  ];

  function renderTable(
    overrides: Omit<
      Partial<Parameters<typeof VisitorsTable>[0]>,
      "onSortChange" | "onPageChange"
    > = {},
  ) {
    const props = {
      visitors,
      loading: false,
      emptyLabel: "No visitors.",
      offset: 0,
      limit: 100,
      sort: DEFAULT_VISITOR_SORT,
      ...overrides,
      onSortChange: vi.fn<(sort: VisitorSortState) => void>(),
      onPageChange: vi.fn<(offset: number) => void>(),
    };
    withDashboard(() => <VisitorsTable {...props} />);
    return props;
  }

  it("shows location, pages, duration and session age", () => {
    renderTable();
    const [first, second] = bodyRows();
    expect(first).toHaveTextContent("Osaka, Japan");
    expect(first).toHaveTextContent("4");
    expect(first).toHaveTextContent("1m 35s");
    expect(first).toHaveTextContent("10m");
    expect(second?.querySelector("td")).toHaveTextContent("—");
  });

  it("asks the parent to sort, starting a new column descending", async () => {
    const props = renderTable();
    await userEvent.click(screen.getByRole("button", { name: "Pages Viewed" }));
    expect(props.onSortChange).toHaveBeenCalledWith({ orderBy: "pages_viewed", direction: "desc" });
    await userEvent.click(screen.getByRole("button", { name: /Last View At/ }));
    expect(props.onSortChange).toHaveBeenLastCalledWith({
      orderBy: "last_view_at",
      direction: "asc",
    });
  });

  it("pages by offset", async () => {
    const props = renderTable({ offset: 100, nextOffset: 200 });
    expect(screen.getByText("Showing 101-102 visitors")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    await userEvent.click(screen.getByRole("button", { name: "Previous" }));
    expect(props.onPageChange.mock.calls).toEqual([[200], [0]]);
  });

  it("disables next on the last page and opens a visitor", async () => {
    const onVisitorSelect = vi.fn();
    renderTable({ onVisitorSelect });
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "View journey for visitor bbb" }));
    expect(onVisitorSelect).toHaveBeenCalledWith(visitors[1]);
  });
});
