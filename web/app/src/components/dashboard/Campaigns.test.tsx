import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../../shared/test/api";
import { testSession } from "../../../../shared/test/fixtures";
import type { CampaignListItem, CampaignsResponse } from "../../lib/campaignsApi";
import Campaigns from "./Campaigns";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";

afterEach(() => vi.restoreAllMocks());

const none = { ratio: null };

function campaign(overrides: Partial<CampaignListItem> = {}): CampaignListItem {
  return {
    id: 7,
    source: "newsletter",
    medium: "email",
    campaign: "launch",
    first_seen_at: 1_700_000_000,
    rules: [],
    visitors: 400,
    converting_visitors: 8,
    revenue: 120_000,
    conversion_rate: 0.02,
    conversion_ci: { lower: 0.01, upper: 0.04 },
    revenue_per_visitor: 300,
    spend: 40_000,
    roas: 3,
    roi: 2,
    spend_entries: [
      {
        id: 1,
        kind: "once",
        start_date: "2026-01-02",
        end_date: "2026-01-02",
        amount: 40_000,
        spent_to_date: 40_000,
      },
    ],
    visitors_change: none,
    revenue_change: none,
    conversion_rate_change: none,
    referrers: [{ label: "mail.google.com", visitors: 30 }],
    landing_paths: [{ id: 3, path: "/launch", count: 300 }],
    countries: [],
    devices: [],
    contents: [],
    terms: [],
    ...overrides,
  };
}

const response: CampaignsResponse = {
  campaigns: [campaign()],
  suggestions: [{ kind: "ref", value: "producthunt", visitors: 55 }],
};

const READ = "/v2/read/projects/42/campaigns";
const WRITE = "/v2/session/projects/42/campaigns";

function renderCampaigns(
  props: { session?: ReturnType<typeof testSession>; publicAccess?: boolean } = {
    session: testSession(),
  },
) {
  let state!: ReturnType<typeof useDashboardState>;
  const Capture = () => {
    state = useDashboardState();
    return null;
  };
  render(() => (
    <DashboardStateProvider>
      <Capture />
      <Campaigns projectId="42" {...props} />
    </DashboardStateProvider>
  ));
  return { state: () => state };
}

const row = () =>
  screen.getByRole("button", { name: /details for launch/ }).closest("tr") as HTMLElement;

describe("Campaigns", () => {
  it("loads campaigns for the dashboard range with the session", async () => {
    const loads = mockApi("get", READ, response);
    renderCampaigns();
    expect(await screen.findByText("launch")).toBeInTheDocument();
    expect(loads[0]?.credentials).toBe("include");
    expect(Object.keys(queryOf(loads[0]))).toEqual(["start", "end"]);
    expect(row()).toHaveTextContent("$40");
    expect(row()).toHaveTextContent("$120");
    expect(row()).toHaveTextContent("3.00×");
    expect(row()).toHaveTextContent("200%");
  });

  it("loads the public demo without the session", async () => {
    const loads = mockApi("get", READ, response);
    renderCampaigns({ publicAccess: true });
    await screen.findByText("launch");
    expect(loads[0]?.credentials).toBe("omit");
  });

  it("reloads when a segment filter changes, and warns that ROAS is skewed", async () => {
    const loads = mockApi("get", READ, response);
    const { state } = renderCampaigns();
    await screen.findByText("launch");
    state().addCountryFilter("DE", "Germany");
    await vi.waitFor(() => expect(loads).toHaveLength(2));
    expect(queryOf(loads[1])).toMatchObject({ country: "DE" });
    expect(
      screen.getByRole("button", { name: "Warning: ROAS is skewed by dashboard filters" }),
    ).toBeInTheDocument();
  });

  it("expands a campaign to its definition and breakdowns", async () => {
    mockApi("get", READ, response);
    renderCampaigns();
    await screen.findByText("launch");
    await userEvent.click(row());
    expect(screen.getByText("Matches visits tagged with")).toBeInTheDocument();
    expect(screen.getByText("8 of 400")).toBeInTheDocument();
    expect(screen.getByText("mail.google.com")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide details for launch" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("filters the dashboard to a campaign", async () => {
    mockApi("get", READ, response);
    const { state } = renderCampaigns();
    await screen.findByText("launch");
    await userEvent.click(row());
    await userEvent.click(
      screen.getByRole("button", { name: "Filter dashboard to this campaign" }),
    );
    expect(state().segments().sources).toEqual([
      { kind: "campaign", id: 7, name: expect.stringContaining("launch") },
    ]);
  });

  it("turns a suggestion into a new campaign", async () => {
    mockApi("get", READ, response);
    mockApi("post", "/v2/read/projects/42/campaign-preview", {
      unclassified: 55,
      direct: 0,
      referrer: 55,
      already_classified: 0,
    });
    const saves = mockApi("post", WRITE, { id: 8, assigned: 55 });
    renderCampaigns();
    await userEvent.click(await screen.findByRole("button", { name: /producthunt/ }));
    expect(screen.getByLabelText("Name")).toHaveValue("producthunt");
    await userEvent.click(screen.getByRole("button", { name: "Save campaign" }));
    await vi.waitFor(() => expect(saves).toHaveLength(1));
    expect(saves[0]?.body).toEqual({
      name: "producthunt",
      rules: [{ param_key: "ref", param_value: "producthunt" }],
    });
  });

  it("refuses to save a campaign without a name or condition", async () => {
    mockApi("get", READ, response);
    const saves = mockApi("post", WRITE, {});
    renderCampaigns();
    await userEvent.click(await screen.findByRole("button", { name: "+ New campaign" }));
    await userEvent.click(screen.getByRole("button", { name: "Save campaign" }));
    expect(screen.getByText("Give the campaign a name.")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Name"), "Launch");
    await userEvent.click(screen.getByRole("button", { name: "Save campaign" }));
    expect(screen.getByText("Add a value for at least one condition.")).toBeInTheDocument();
    expect(saves).toHaveLength(0);
  });

  it("deletes a campaign after confirmation", async () => {
    mockApi("get", READ, response);
    const deletes = mockApi("delete", WRITE, {});
    const confirm = vi
      .spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    renderCampaigns();
    await screen.findByText("launch");
    await userEvent.click(row());
    await userEvent.click(screen.getByRole("button", { name: "Delete campaign" }));
    expect(deletes).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Delete campaign" }));
    await vi.waitFor(() => expect(deletes).toHaveLength(1));
    expect(queryOf(deletes[0])).toEqual({ id: "7" });
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it("is read-only on the public demo", async () => {
    mockApi("get", READ, response);
    renderCampaigns({ publicAccess: true });
    await screen.findByText("launch");
    await userEvent.click(row());
    expect(screen.queryByRole("button", { name: "Delete campaign" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "+ New campaign" }));
    expect(screen.getByRole("button", { name: "Save campaign" })).toBeDisabled();
  });

  it("shows a load error", async () => {
    mockApi("get", READ, { error: "Campaigns failed." }, 500);
    renderCampaigns();
    expect(await screen.findByText("Campaigns failed.")).toBeInTheDocument();
  });

  it("explains an empty period", async () => {
    mockApi("get", READ, { campaigns: [], suggestions: [] });
    renderCampaigns();
    expect(await screen.findByText(/No campaign traffic in this period/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /details for/ })).toBeNull();
  });
});
