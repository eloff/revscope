import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AuthSession } from "../../../../shared/src/authState";
import { projectOptions } from "../../../../shared/src/projectSelection";
import { mockApi, queryOf } from "../../../../shared/test/api";
import { testProject, testSession } from "../../../../shared/test/fixtures";
import { testBucket, testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import { shortcutToRange } from "../../lib/dateRange";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import ProjectsOverview from "./ProjectsOverview";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const PROJECTS = { alpha_com: testProject("7"), beta_com: testProject("9", "Asia/Tokyo") };

const overview = (visitors: number, totalRevenue: number) =>
  testOverview({
    current_metrics: testPeriodMetrics({ visitors, total_revenue: totalRevenue }),
    buckets: [testBucket({ visitors })],
  });

function renderOverview(session: AuthSession, onOpenProject = vi.fn()) {
  render(() => (
    <DashboardStateProvider>
      <ProjectsOverview
        session={session}
        projects={projectOptions(session.projects)}
        currentProjectId="7"
        cache={new Map()}
        onOpenProject={onOpenProject}
      />
    </DashboardStateProvider>
  ));
  return { onOpenProject };
}

function card(name: string): HTMLElement {
  const article = screen.getByRole("button", { name: new RegExp(`^${name}`) }).closest("article");
  if (!article) {
    throw new Error(`no card for ${name}`);
  }
  return article;
}

describe("ProjectsOverview", () => {
  it("shows every project's visitors and revenue, and opens one", async () => {
    const alpha = mockApi("get", "/v2/read/projects/7/overview", overview(1234, 0));
    const beta = mockApi("get", "/v2/read/projects/9/overview", overview(5, 0));
    const session = testSession({ plan: "growth", projects: PROJECTS });
    const { onOpenProject } = renderOverview(session);

    expect(screen.getByRole("heading", { name: "2 projects" })).toBeInTheDocument();
    await vi.waitFor(() =>
      expect(within(card("alpha_com")).getByText((1234).toLocaleString())).toBeInTheDocument(),
    );
    expect(within(card("beta_com")).getByText("5")).toBeInTheDocument();
    expect(alpha[0]?.credentials).toBe("include");
    expect(beta).toHaveLength(1);
    // The default preset, worked out on each card's own project clock.
    const tokyo = shortcutToRange("last_30_days", "Asia/Tokyo");
    expect(queryOf(beta[0])).toMatchObject({ start: String(tokyo.start), end: String(tokyo.end) });
    const utc = shortcutToRange("last_30_days", "UTC");
    expect(queryOf(alpha[0])).toMatchObject({ start: String(utc.start), end: String(utc.end) });

    await userEvent.setup().click(screen.getByRole("button", { name: /^beta_com/ }));
    expect(onOpenProject).toHaveBeenCalledWith("9");
  });

  it("says when a project has nothing in the period", async () => {
    mockApi("get", "/v2/read/projects/7/overview", testOverview());
    mockApi("get", "/v2/read/projects/9/overview", overview(3, 0));
    renderOverview(testSession({ plan: "growth", projects: PROJECTS }));
    expect(
      await within(card("alpha_com")).findByText("No visitors or revenue in this period."),
    ).toBeInTheDocument();
  });

  it("adds a project named after the website", async () => {
    mockApi("get", "/v2/read/projects/42/overview", overview(1, 0));
    const created = mockApi("post", "/v2/session/projects", { id: "50", name: "shopexamplecom" });
    const me = mockApi("get", "/v2/session/me", testSession({ plan: "growth" }));
    const { onOpenProject } = renderOverview(testSession({ plan: "growth" }));
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "Add project" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "https://Shop.Example.com/");
    expect(within(dialog).getByText("Project name: shopexamplecom")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Add project" }));

    expect(await within(dialog).findByText("Added shopexamplecom")).toBeInTheDocument();
    expect(created[0]?.body).toMatchObject({ website: "https://Shop.Example.com/" });
    expect(me).toHaveLength(1);
    await user.click(within(dialog).getByRole("button", { name: "Open project" }));
    expect(onOpenProject).toHaveBeenCalledWith("50");
  });

  it("shows the server's refusal to add a project", async () => {
    mockApi("get", "/v2/read/projects/42/overview", overview(1, 0));
    mockApi(
      "post",
      "/v2/session/projects",
      { error: "A project with this name already exists." },
      409,
    );
    renderOverview(testSession({ plan: "growth" }));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add project" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "taken.com");
    await user.click(within(dialog).getByRole("button", { name: "Add project" }));
    expect(
      await within(dialog).findByText("A project with this name already exists."),
    ).toBeInTheDocument();
  });

  it("points a single-website plan at the upgrade instead", () => {
    mockApi("get", "/v2/read/projects/7/overview", overview(1, 0));
    mockApi("get", "/v2/read/projects/9/overview", overview(1, 0));
    renderOverview(testSession({ plan: "starter", projects: PROJECTS }));
    expect(screen.queryByRole("button", { name: "Add project" })).toBeNull();
    expect(screen.getByRole("link", { name: "Upgrade to Growth" })).toHaveAttribute(
      "href",
      "/billing",
    );
  });
});
