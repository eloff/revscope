import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../../shared/test/api";
import { testSession } from "../../../../shared/test/fixtures";
import { lastChart } from "../../../test/apexchartsMock";
import type { Funnel, FunnelDataResponse } from "../../lib/funnelsApi";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";
import Funnels from "./Funnels";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => vi.restoreAllMocks());

const none = { ratio: null };
const READ = "/v2/read/projects/42";
const WRITE = "/v2/session/projects/42/funnels";

function funnel(id: number, name: string): Funnel {
  return {
    id,
    name,
    steps: [
      {
        id: id * 10,
        order: 0,
        step_type: "pageview",
        path_id: 1,
        path: "/",
        is_landing: false,
        payment_types: [],
      },
      {
        id: id * 10 + 1,
        order: 1,
        step_type: "pageview",
        path_id: 2,
        path: "/pricing",
        is_landing: false,
        payment_types: [],
      },
    ],
  };
}

function funnelData(funnelId: number, visitors: [number, number]): FunnelDataResponse {
  return {
    funnel_id: funnelId,
    steps: [
      {
        step_id: 1,
        order: 0,
        step_type: "pageview",
        name: "Home",
        path: "/",
        visitors: visitors[0],
        previous_visitors: 0,
        visitors_change: none,
      },
      {
        step_id: 2,
        order: 1,
        step_type: "pageview",
        name: "Pricing",
        path: "/pricing",
        visitors: visitors[1],
        previous_visitors: 0,
        visitors_change: none,
        dropoff_rate: 1 - visitors[1] / visitors[0],
      },
    ],
  };
}

const signup = funnel(1, "Signup");
const upgrade = funnel(2, "Upgrade");

function renderFunnels(
  props: { session?: ReturnType<typeof testSession>; publicAccess?: boolean } = {
    session: testSession(),
  },
) {
  render(() => (
    <DashboardStateProvider>
      <Funnels projectId="42" {...props} />
    </DashboardStateProvider>
  ));
}

describe("Funnels", () => {
  it("bootstraps the funnel list and the first funnel's data in one request", async () => {
    const boots = mockApi("get", `${READ}/funnel-data`, {
      funnels: [signup, upgrade],
      data: funnelData(1, [100, 40]),
    });
    renderFunnels();
    expect(await screen.findByRole("tab", { name: "Signup" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(boots).toHaveLength(1);
    expect(queryOf(boots[0])).not.toHaveProperty("funnel_id");
    await vi.waitFor(() => expect(lastChart().options.chart?.type).toBe("funnel"));
  });

  it("loads another funnel's data when its tab is chosen, then serves it from cache", async () => {
    const calls = mockApi("get", `${READ}/funnel-data`, ({ url }) =>
      url.searchParams.has("funnel_id")
        ? HttpResponse.json(funnelData(2, [50, 10]))
        : HttpResponse.json({ funnels: [signup, upgrade], data: funnelData(1, [100, 40]) }),
    );
    renderFunnels();
    await userEvent.click(await screen.findByRole("tab", { name: "Upgrade" }));
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    expect(queryOf(calls[1])).toMatchObject({ funnel_id: "2" });
    await userEvent.click(screen.getByRole("tab", { name: "Signup" }));
    await userEvent.click(screen.getByRole("tab", { name: "Upgrade" }));
    expect(calls).toHaveLength(2);
  });

  it("keeps the selected funnel's data when an earlier request finishes last", async () => {
    let releaseUpgrade!: () => void;
    const upgradeGate = new Promise<void>((resolve) => {
      releaseUpgrade = resolve;
    });
    const third = { ...funnel(3, "Trial"), steps: funnel(3, "Trial").steps };
    mockApi("get", `${READ}/funnel-data`, async ({ url }) => {
      const id = url.searchParams.get("funnel_id");
      if (id === "2") {
        await upgradeGate;
        return HttpResponse.json(funnelData(2, [50, 10]));
      }
      if (id === "3") {
        return HttpResponse.json(funnelData(3, [70, 7]));
      }
      return HttpResponse.json({
        funnels: [signup, upgrade, third],
        data: funnelData(1, [100, 40]),
      });
    });
    renderFunnels();
    await userEvent.click(await screen.findByRole("tab", { name: "Upgrade" }));
    await userEvent.click(screen.getByRole("tab", { name: "Trial" }));
    // The chart draws normalized widths: Trial (70 → 7) is [1, 0.15], Upgrade (50 → 10) [1, 0.2].
    const widths = () =>
      (lastChart().options.series as { data: number[] }[] | undefined)?.[0]?.data;
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.15]));
    releaseUpgrade();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(widths()).toEqual([1, 0.15]);
    expect(screen.getByRole("tab", { name: "Trial" })).toHaveAttribute("aria-selected", "true");
  });

  it("says when there are no funnels yet", async () => {
    mockApi("get", `${READ}/funnel-data`, { funnels: [] });
    renderFunnels();
    expect(await screen.findByText(/No funnels yet/)).toBeInTheDocument();
  });

  it("shows a load error", async () => {
    mockApi("get", `${READ}/funnel-data`, { error: "Funnels failed." }, 500);
    renderFunnels();
    expect((await screen.findAllByText("Funnels failed.")).length).toBeGreaterThan(0);
  });

  it("creates a funnel from pages and saves it", async () => {
    mockApi("get", `${READ}/funnel-data`, { funnels: [signup], data: funnelData(1, [100, 40]) });
    mockApi("get", `${READ}/funnel-options`, {
      paths: [
        { id: 1, name: "/" },
        { id: 2, name: "/pricing" },
      ],
      events: [{ id: 5, name: "cta", path_ids: [2] }],
    });
    mockApi("post", `${READ}/funnel-preview`, funnelData(0, [80, 20]));
    const saved = funnel(9, "Pricing path");
    const saves = mockApi("post", WRITE, { funnel: saved, data: funnelData(9, [80, 20]) });
    renderFunnels();
    await userEvent.click(await screen.findByRole("button", { name: "+ Create funnel" }));
    expect(await screen.findByRole("heading", { name: "Create funnel" })).toBeInTheDocument();

    const name = screen.getByLabelText("Name");
    await userEvent.clear(name);
    await userEvent.type(name, "Pricing path");
    await userEvent.click(await screen.findByRole("button", { name: "Add step" }));
    const pages = screen.getAllByLabelText("Page");
    await userEvent.selectOptions(pages[1] as HTMLElement, "2");
    const events = screen.getAllByLabelText("Event (optional)");
    expect(
      within(events[1] as HTMLElement)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["No event", "cta"]);
    await userEvent.click(screen.getByRole("button", { name: "Save funnel" }));

    expect(screen.queryByText(/Each step needs a page/)).toBeNull();
    await vi.waitFor(() => expect(saves).toHaveLength(1));
    expect(saves[0]?.body).toMatchObject({
      name: "Pricing path",
      steps: [
        { step_type: "pageview", path_id: 1 },
        { step_type: "pageview", path_id: 2 },
      ],
    });
    expect(await screen.findByRole("tab", { name: "Pricing path" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("refuses to save without a name", async () => {
    mockApi("get", `${READ}/funnel-data`, { funnels: [signup], data: funnelData(1, [100, 40]) });
    mockApi("get", `${READ}/funnel-options`, { paths: [{ id: 1, name: "/" }], events: [] });
    mockApi("post", `${READ}/funnel-preview`, funnelData(0, [1, 1]));
    renderFunnels();
    await userEvent.click(await screen.findByRole("button", { name: "+ Create funnel" }));
    await userEvent.clear(await screen.findByLabelText("Name"));
    await userEvent.click(screen.getByRole("button", { name: "Save funnel" }));
    expect(screen.getByText("Name the funnel before saving.")).toBeInTheDocument();
  });

  it("deletes the selected funnel after confirmation", async () => {
    mockApi("get", `${READ}/funnel-data`, ({ url }) =>
      url.searchParams.has("funnel_id")
        ? HttpResponse.json(funnelData(2, [5, 1]))
        : HttpResponse.json({ funnels: [signup, upgrade], data: funnelData(1, [100, 40]) }),
    );
    const deletes = mockApi("delete", WRITE, { funnels: [upgrade] });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderFunnels();
    await screen.findByRole("tab", { name: "Signup" });
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await vi.waitFor(() => expect(deletes).toHaveLength(1));
    expect(queryOf(deletes[0])).toEqual({ funnel_id: "1" });
    expect(await screen.findByRole("tab", { name: "Upgrade" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.queryByRole("tab", { name: "Signup" })).toBeNull();
  });

  it("is read-only on the public demo", async () => {
    const boots = mockApi("get", `${READ}/funnel-data`, {
      funnels: [signup],
      data: funnelData(1, [100, 40]),
    });
    renderFunnels({ publicAccess: true });
    await screen.findByRole("tab", { name: "Signup" });
    expect(boots[0]?.credentials).toBe("omit");
    expect(screen.queryByRole("button", { name: "+ Create funnel" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  });

  it("doesn't cache a funnel's data for a range the viewer has left", async () => {
    let releaseOld!: () => void;
    const oldGate = new Promise<void>((resolve) => {
      releaseOld = resolve;
    });
    const upgradeRequests: string[] = [];
    mockApi("get", `${READ}/funnel-data`, async ({ url }) => {
      if (url.searchParams.get("funnel_id") === "2") {
        const start = url.searchParams.get("start") ?? "";
        upgradeRequests.push(start);
        if (upgradeRequests.length === 1) {
          await oldGate;
          return HttpResponse.json(funnelData(2, [50, 10]));
        }
        return HttpResponse.json(funnelData(2, [100, 50]));
      }
      return HttpResponse.json({ funnels: [signup, upgrade], data: funnelData(1, [100, 40]) });
    });
    let state!: ReturnType<typeof useDashboardState>;
    const Capture = () => {
      state = useDashboardState();
      return null;
    };
    render(() => (
      <DashboardStateProvider>
        <Capture />
        <Funnels projectId="42" session={testSession()} />
      </DashboardStateProvider>
    ));
    await userEvent.click(await screen.findByRole("tab", { name: "Upgrade" }));
    await vi.waitFor(() => expect(upgradeRequests).toHaveLength(1));
    const current = state.range();
    state.setRange({ start: current.start - 86_400, end: current.end });
    await vi.waitFor(() => expect(upgradeRequests).toHaveLength(2));
    releaseOld();
    await new Promise((resolve) => setTimeout(resolve, 50));

    await userEvent.click(screen.getByRole("tab", { name: "Signup" }));
    await userEvent.click(screen.getByRole("tab", { name: "Upgrade" }));
    const widths = () =>
      (lastChart().options.series as { data: number[] }[] | undefined)?.[0]?.data;
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.5]));
  });

  it("doesn't cache a pre-save data response over the saved funnel", async () => {
    let releaseOld!: () => void;
    const oldGate = new Promise<void>((resolve) => {
      releaseOld = resolve;
    });
    let signupRequests = 0;
    mockApi("get", `${READ}/funnel-data`, async ({ url }) => {
      const id = url.searchParams.get("funnel_id");
      if (id === "1") {
        signupRequests += 1;
        await oldGate;
        return HttpResponse.json(funnelData(1, [100, 90]));
      }
      if (id === "2") {
        return HttpResponse.json(funnelData(2, [100, 60]));
      }
      return HttpResponse.json({ funnels: [signup, upgrade], data: funnelData(1, [100, 40]) });
    });
    mockApi("get", `${READ}/funnel-options`, {
      paths: [
        { id: 1, name: "/" },
        { id: 2, name: "/pricing" },
      ],
      events: [],
    });
    mockApi("post", `${READ}/funnel-preview`, funnelData(0, [100, 30]));
    mockApi("post", WRITE, { funnel: signup, data: funnelData(1, [100, 30]) });
    let state!: ReturnType<typeof useDashboardState>;
    const Capture = () => {
      state = useDashboardState();
      return null;
    };
    render(() => (
      <DashboardStateProvider>
        <Capture />
        <Funnels projectId="42" session={testSession()} />
      </DashboardStateProvider>
    ));
    await screen.findByRole("tab", { name: "Signup" });
    const current = state.range();
    state.setRange({ start: current.start - 86_400, end: current.end });
    await vi.waitFor(() => expect(signupRequests).toBe(1));

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    await userEvent.click(await screen.findByRole("button", { name: "Save funnel" }));
    const widths = () =>
      (lastChart().options.series as { data: number[] }[] | undefined)?.[0]?.data;
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.3]));
    releaseOld();
    await new Promise((resolve) => setTimeout(resolve, 50));

    await userEvent.click(screen.getByRole("tab", { name: "Upgrade" }));
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.6]));
    await userEvent.click(screen.getByRole("tab", { name: "Signup" }));
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.3]));
  });

  it("refetches a saved funnel's data under active filters instead of caching the unfiltered save", async () => {
    const dataRequests: URL[] = [];
    mockApi("get", `${READ}/funnel-data`, ({ url }) => {
      if (url.searchParams.get("funnel_id") === "1") {
        dataRequests.push(url);
        return HttpResponse.json(funnelData(1, [100, 70]));
      }
      return HttpResponse.json({ funnels: [signup], data: funnelData(1, [100, 40]) });
    });
    mockApi("get", `${READ}/funnel-options`, {
      paths: [
        { id: 1, name: "/" },
        { id: 2, name: "/pricing" },
      ],
      events: [],
    });
    mockApi("post", `${READ}/funnel-preview`, funnelData(0, [100, 20]));
    // The save's own data ignores segment filters.
    mockApi("post", WRITE, { funnel: signup, data: funnelData(1, [100, 20]) });
    let state!: ReturnType<typeof useDashboardState>;
    const Capture = () => {
      state = useDashboardState();
      return null;
    };
    render(() => (
      <DashboardStateProvider>
        <Capture />
        <Funnels projectId="42" session={testSession()} />
      </DashboardStateProvider>
    ));
    await screen.findByRole("tab", { name: "Signup" });
    state.addCountryFilter("DE", "Germany");
    await vi.waitFor(() => expect(dataRequests).toHaveLength(1));

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    await userEvent.click(await screen.findByRole("button", { name: "Save funnel" }));
    await vi.waitFor(() => expect(dataRequests).toHaveLength(2));
    expect(dataRequests[1]?.searchParams.getAll("country")).toEqual(["DE"]);
    const widths = () =>
      (lastChart().options.series as { data: number[] }[] | undefined)?.[0]?.data;
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.7]));
  });

  it("refetches the shown funnel when the range changes instead of serving the old range", async () => {
    const dataRequests: string[] = [];
    mockApi("get", `${READ}/funnel-data`, ({ url }) => {
      if (url.searchParams.get("funnel_id") === "1") {
        dataRequests.push(url.searchParams.get("start") ?? "");
        return HttpResponse.json(funnelData(1, [100, 70]));
      }
      return HttpResponse.json({ funnels: [signup], data: funnelData(1, [100, 40]) });
    });
    let state!: ReturnType<typeof useDashboardState>;
    const Capture = () => {
      state = useDashboardState();
      return null;
    };
    render(() => (
      <DashboardStateProvider>
        <Capture />
        <Funnels projectId="42" session={testSession()} />
      </DashboardStateProvider>
    ));
    await screen.findByRole("tab", { name: "Signup" });
    const current = state.range();
    state.setRange({ start: current.start - 86_400, end: current.end });
    await vi.waitFor(() => expect(dataRequests).toEqual([String(current.start - 86_400)]));
    const widths = () =>
      (lastChart().options.series as { data: number[] }[] | undefined)?.[0]?.data;
    await vi.waitFor(() => expect(widths()).toEqual([1, 0.7]));
  });
});
