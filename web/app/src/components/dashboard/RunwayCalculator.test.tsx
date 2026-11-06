import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { lastChart } from "../../../test/apexchartsMock";
import type { PaymentActivityResponse } from "../../lib/paymentActivityApi";
import RunwayCalculator from "./RunwayCalculator";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

/**
 * The text box in the field whose label starts with `label`. Each label wraps
 * its hint (and, for growth, the window select), so match on the prefix.
 */
function input(label: string): HTMLInputElement {
  const field = screen
    .getAllByText(label)
    .map((text) => text.closest("label"))
    .find((element) => element !== null);
  const box = field?.querySelector<HTMLInputElement>('input[type="text"]');
  if (!box) throw new Error(`no input for ${label}`);
  return box;
}

async function setInput(label: string, value: string) {
  const user = userEvent.setup();
  await user.clear(input(label));
  if (value) {
    await user.type(input(label), value);
  }
}

/** The value shown on a summary card, found by its label. */
function summary(label: string): string {
  return screen.getByText(label).nextElementSibling?.textContent ?? "";
}

describe("RunwayCalculator (standalone)", () => {
  it("starts from editable example numbers", () => {
    render(() => <RunwayCalculator standalone />);
    expect(input("Current cash balance")).toHaveValue("250,000");
    expect(input("Monthly revenue")).toHaveValue("10,000");
    expect(input("Monthly burn rate")).toHaveValue("40,000");
    expect(input("Monthly revenue growth")).toHaveValue("10");
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(summary("Monthly net burn")).toBe("$30,000");
    expect(lastChart().history.length).toBeGreaterThan(1);
  });

  it("warns when cash runs out within six months", async () => {
    render(() => <RunwayCalculator standalone />);
    await setInput("Current cash balance", "50,000");
    await setInput("Monthly revenue", "0");
    await setInput("Monthly revenue growth", "0");
    expect(summary("Runway")).toBe("1.3 mo");
    expect(summary("Break-even")).toBe("—");
    expect(screen.getByText("Act now")).toBeInTheDocument();
  });

  it.each([
    // A $30k net burn: 10 months, then 15 months of cash.
    ["300,000", "Begin fundraising"],
    ["450,000", "Start preparing"],
    ["2,000,000", "Plenty of runway"],
  ])("advises on %s of cash with no growth: %s", async (cash, title) => {
    render(() => <RunwayCalculator standalone />);
    await setInput("Monthly revenue growth", "0");
    await setInput("Current cash balance", cash);
    expect(screen.getByText(title)).toBeInTheDocument();
  });

  it("calls a profitable company default alive", async () => {
    render(() => <RunwayCalculator standalone />);
    await setInput("Monthly burn rate", "5,000");
    expect(summary("Runway")).toBe("∞");
    expect(summary("Monthly net burn")).toBe("+$5,000");
    expect(summary("Break-even")).toBe("Now");
    expect(screen.getByText("Default alive")).toBeInTheDocument();
  });

  it("asks for cash and burn before projecting", async () => {
    render(() => <RunwayCalculator standalone />);
    await setInput("Current cash balance", "");
    expect(
      screen.getByText(/Enter your current cash balance and monthly burn rate/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Runway")).toBeNull();
  });
});

describe("RunwayCalculator (project)", () => {
  const data: PaymentActivityResponse = {
    start: 0,
    end: 0,
    days: [],
    monthly_revenue: Array.from({ length: 12 }, (_, monthsAgo) => ({
      months_ago: monthsAgo,
      // 10% month-over-month growth, most recent first, in thousandths.
      revenue: Math.round(20_000_000 / 1.1 ** monthsAgo),
    })),
  };

  it("fills revenue and growth from the project's history", () => {
    render(() => <RunwayCalculator data={data} />);
    expect(input("Monthly revenue")).toHaveValue("20000");
    expect(screen.getByText("From last 30 days")).toBeInTheDocument();
    expect(input("Monthly revenue growth")).toHaveValue("10.0");
    // 1.1^12 compounds to +214% a year.
    expect(screen.getByText("Trend over last 6 months (214% annualized)")).toBeInTheDocument();
    expect(screen.getByRole("combobox")).toHaveValue("6");
  });

  it("marks edits as custom and resets them to the history", async () => {
    render(() => <RunwayCalculator data={data} />);
    await setInput("Monthly revenue growth", "2");
    expect(screen.getByRole("combobox")).toHaveValue("custom");
    expect(screen.getByText(/annualized$/)).toHaveTextContent("26.8% annualized");
    await userEvent.click(screen.getByRole("button", { name: /Reset revenue & growth/ }));
    expect(input("Monthly revenue growth")).toHaveValue("10.0");
    expect(screen.queryByRole("button", { name: /Reset/ })).toBeNull();
  });

  it("shows loading and errors for the history", () => {
    render(() => <RunwayCalculator loading error="Revenue history failed." />);
    expect(screen.getByText("Loading recent revenue...")).toBeInTheDocument();
    expect(screen.getByText("Revenue history failed.")).toBeInTheDocument();
    expect(
      screen.getByText("Not enough revenue history yet to estimate a trend"),
    ).toBeInTheDocument();
  });
});
