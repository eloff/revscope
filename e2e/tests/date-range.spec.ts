import { expect, test } from "./support/demo";

test.describe("date range", () => {
  test("opens on a period ending now, so there is no next period", async ({ demo }) => {
    await demo.goto();
    const { start, end } = await demo.dateRange();
    expect(end).toBeGreaterThan(start);
    await expect(demo.page.getByRole("button", { name: "Next period" })).toBeDisabled();
    await expect(demo.page.getByRole("button", { name: "Previous period" })).toBeEnabled();
  });

  test("a wider preset moves the start back and raises the totals", async ({ demo }) => {
    await demo.goto();
    const thirtyDays = await demo.loadedMetric("Visitors");
    const before = await demo.dateRange();

    await demo.choosePreset("Last 90 Days");

    await expect.poll(async () => (await demo.dateRange()).start).toBeLessThan(before.start);
    expect((await demo.dateRange()).end).toBe(before.end);
    await expect.poll(() => demo.metric("Visitors")).toBeGreaterThan(thirtyDays);
  });

  test("the current preset is marked in the picker", async ({ demo }) => {
    await demo.goto();
    await demo.choosePreset("Last 60 Days");
    await demo.dateInputs.first().click();
    await expect(demo.page.getByRole("option", { name: "Last 60 Days" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(demo.page.getByRole("option", { name: "Last 90 Days" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  test("Previous and Next shift the period", async ({ demo }) => {
    await demo.goto();
    const original = await demo.dateRange();
    const visitors = await demo.loadedMetric("Visitors");
    const previous = demo.page.getByRole("button", { name: "Previous period" });
    const next = demo.page.getByRole("button", { name: "Next period" });

    await previous.click();
    await expect.poll(async () => (await demo.dateRange()).end).toBeLessThanOrEqual(original.start);
    await expect(next).toBeEnabled();
    // Any total but the current one, since an old local demo can have nothing back there.
    await expect.poll(() => demo.metric("Visitors")).not.toBeNull();
    await expect.poll(() => demo.metric("Visitors")).not.toBe(visitors);

    await next.click();
    await expect.poll(async () => (await demo.dateRange()).end).toBe(original.end);
    expect((await demo.dateRange()).start).toBe(original.start);
    await expect(next).toBeDisabled();
    await expect.poll(() => demo.metric("Visitors")).toBe(visitors);
  });

  test("a range change reloads the open tab", async ({ demo }) => {
    await demo.goto("?tab=pages");
    const grid = demo.grid("Pages", "Exit rate");
    await grid.expectRows();
    await grid.sortBy("Visitors", "descending");
    await grid.expectSorted("Visitors", "descending");
    const thirtyDays = await grid.columnSum("Visitors");

    await demo.choosePreset("Last 90 Days");

    await expect.poll(() => grid.columnSum("Visitors")).toBeGreaterThan(thirtyDays);
  });

  test("Revenue uses its own fixed window, so the controls are off there", async ({ demo }) => {
    await demo.goto("?tab=revenue");
    await expect(demo.dateInputs.first()).toBeDisabled();
    await expect(demo.page.getByRole("button", { name: "Previous period" })).toBeDisabled();
    await expect(
      demo.panel("Revenue").getByRole("heading", { name: "Purchase activity" }),
    ).toBeVisible();

    await demo.tab("Overview").click();
    await expect(demo.dateInputs.first()).toBeEnabled();
  });
});
