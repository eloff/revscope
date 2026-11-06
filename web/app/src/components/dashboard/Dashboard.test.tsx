import { selectedProjectId, setSelectedProjectId } from "@revscope/web-shared/projectSelection";
import { cleanup, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { mockApi, queryOf } from "../../../../shared/test/api";
import { signIn, signOut } from "../../../../shared/test/auth";
import { testProject, testSession } from "../../../../shared/test/fixtures";
import { stubLocation } from "../../../../shared/test/location";
import { testOverview } from "../../../test/dashboardFixtures";
import Dashboard from "./Dashboard";
import Demo from "./Demo";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const build = vi.hoisted(() => ({ selfHosted: false }));
vi.mock("@revscope/web-shared/links", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@revscope/web-shared/links")>()),
  get SELF_HOSTED() {
    return build.selfHosted;
  },
}));

afterEach(async () => {
  // Unmount first: resetting the project or session under a mounted dashboard
  // makes it fetch another project after this test's handlers are gone.
  cleanup();
  window.history.replaceState(null, "", "/");
  setSelectedProjectId("");
  await signOut();
});

function mockProject(id: string) {
  mockApi("get", "/v2/public/chat/status", { available: false });
  mockApi("get", "/v2/read/projects/:id/imported-coverage", {
    include_in_reports: true,
    has_data: false,
    has_events: false,
    ranges: [],
  });
  return mockApi("get", `/v2/read/projects/${id}/overview`, testOverview());
}

describe("Dashboard", () => {
  it("shows the public demo to a visitor without a session", async () => {
    const loads = mockProject("1");
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    expect(loads[0]?.credentials).toBe("omit");
  });

  it("asks a self-hosted visitor without a session to log in", async () => {
    build.selfHosted = true;
    onTestFinished(() => {
      build.selfHosted = false;
    });
    onTestFinished(() => {
      vi.unstubAllGlobals();
    });
    const assign = stubLocation("/ui/dashboard", "?project=42");
    mockApi("get", "/v2/public/auth/signup", { open: false });
    mockApi("post", "/v2/public/auth/login", testSession());
    // Logging in saves the session before redirecting, so the account's project mounts.
    mockProject("42");
    render(() => <Dashboard />);
    const dialog = await screen.findByRole("dialog", { name: "Log in" });
    expect(screen.getByText("Log in to view analytics.")).toBeTruthy();
    await userEvent.type(within(dialog).getByLabelText("Email"), "ada@example.com");
    await userEvent.type(within(dialog).getByLabelText("Password"), "correctHorse9!");
    await userEvent.click(within(dialog).getByRole("button", { name: "Log in" }));
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/ui/dashboard?project=42"));
  });

  it("signs out and shows the demo when the API and /me both refuse the cached session", async () => {
    signIn(testSession({ projects: { a_com: testProject("7") } }));
    mockApi("get", "/v2/read/projects/7/overview", { error: "invalid session" }, 401);
    mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    const demo = mockProject("1");
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(demo).toHaveLength(1));
    expect(demo[0]?.credentials).toBe("omit");
  });

  it("opens the selected project with the session", async () => {
    signIn(testSession({ projects: { a_com: testProject("7"), b_com: testProject("8") } }));
    setSelectedProjectId("8");
    const loads = mockProject("8");
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    expect(loads[0]?.credentials).toBe("include");
  });

  it("falls back to the account's first project", async () => {
    signIn(testSession({ projects: { a_com: testProject("7") } }));
    setSelectedProjectId("999");
    const loads = mockProject("7");
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(loads).toHaveLength(1));
  });

  it("stays on the fallback project when a project that sorts first is added", async () => {
    signIn(testSession({ plan: "growth", projects: { m_com: testProject("7") } }));
    const loads = mockProject("7");
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    expect(selectedProjectId()).toBe("7");
    signIn(
      testSession({
        plan: "growth",
        projects: { a_com: testProject("3"), m_com: testProject("7") },
      }),
    );
    expect(selectedProjectId()).toBe("7");
  });

  it("opens a project from the Projects tab on the period its card showed", async () => {
    // Already the next day in Tokyo, so its default period differs from the card's.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-03-15T20:00:00Z"));
    onTestFinished(() => {
      vi.useRealTimers();
    });
    signIn(
      testSession({
        plan: "growth",
        projects: { a_com: testProject("7"), b_com: testProject("8", "Asia/Tokyo") },
      }),
    );
    setSelectedProjectId("7");
    window.history.replaceState(null, "", "/?tab=projects");
    mockProject("7");
    const cards = mockApi("get", "/v2/read/projects/8/overview", testOverview());
    render(() => <Dashboard />);
    await vi.waitFor(() => expect(cards).toHaveLength(1));
    const cardQuery = queryOf(cards[0]);
    await userEvent.setup().click(await screen.findByRole("button", { name: /^b_com/ }));
    await vi.waitFor(() => expect(cards.length).toBeGreaterThan(1));
    expect(selectedProjectId()).toBe("8");
    expect(queryOf(cards.at(-1))).toMatchObject({ start: cardQuery.start, end: cardQuery.end });
  });

  it("explains an account with no projects", async () => {
    signIn(testSession({ projects: {} }));
    render(() => <Dashboard />);
    expect(await screen.findByText("This account has no projects yet.")).toBeInTheDocument();
  });

  it("switches project from the account menu and drops the old project's deep link", async () => {
    signIn(testSession({ projects: { a_com: testProject("7"), b_com: testProject("8") } }));
    setSelectedProjectId("7");
    mockProject("7");
    const other = mockProject("8");
    window.history.replaceState(null, "", "/dashboard?tab=overview&path_id=5");
    render(() => <Dashboard />);
    await userEvent.click(await screen.findByRole("button", { name: "Account menu" }));
    await userEvent.click(await screen.findByRole("menuitemradio", { name: "b_com" }));
    expect(selectedProjectId()).toBe("8");
    await vi.waitFor(() => expect(other).toHaveLength(1));
    expect(new URLSearchParams(window.location.search).has("path_id")).toBe(false);
  });
});

describe("Demo", () => {
  it("renders the public demo full screen without the account header", async () => {
    const loads = mockProject("1");
    render(() => <Demo />);
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    expect(screen.queryByRole("button", { name: "Account menu" })).toBeNull();
  });
});
