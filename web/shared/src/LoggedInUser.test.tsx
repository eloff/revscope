import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { signIn, signOut } from "../test/auth";
import { testProject, testSession } from "../test/fixtures";
import { stubLocation } from "../test/location";
import { API, server } from "../test/server";
import AuthButton from "./AuthButton";
import { useAuthState } from "./authState";
import LoggedInUser from "./LoggedInUser";
import { selectedProjectId, setSelectedProjectId } from "./projectSelection";

const twoProjects = testSession({
  name: "  Grace  Brewster Hopper ",
  projects: { beta: testProject("2"), alpha: testProject("1") },
});

let assign: ReturnType<typeof vi.fn>;

beforeEach(() => {
  assign = stubLocation("/settings");
});

afterEach(async () => {
  vi.unstubAllGlobals();
  setSelectedProjectId("");
  await signOut();
});

async function openMenu() {
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Account menu" }));
  await screen.findByRole("menu");
  return user;
}

describe("LoggedInUser", () => {
  it("renders nothing while signed out", () => {
    render(() => <LoggedInUser />);
    expect(screen.queryByRole("button", { name: "Account menu" })).not.toBeInTheDocument();
  });

  it("shows initials, first name and the selected project", async () => {
    signIn(twoProjects);
    setSelectedProjectId("2");
    render(() => <LoggedInUser />);
    const chip = await screen.findByRole("button", { name: "Account menu" });
    expect(chip).toHaveTextContent("GB");
    expect(chip).toHaveTextContent("Grace");
    expect(chip).toHaveTextContent("beta");
  });

  it("shows the gravatar when there is one", async () => {
    signIn(testSession({ gravatar_url: "https://gravatar.test/a.png" }));
    const { container } = render(() => <LoggedInUser />);
    await screen.findByRole("button", { name: "Account menu" });
    expect(container.querySelector("img")).toHaveAttribute("src", "https://gravatar.test/a.png");
  });

  it("falls back to the first project when the stored one isn't on the account", async () => {
    signIn(twoProjects);
    setSelectedProjectId("404");
    render(() => <LoggedInUser />);
    expect(await screen.findByRole("button", { name: "Account menu" })).toHaveTextContent("alpha");
  });

  it("navigates from the menu", async () => {
    signIn();
    render(() => <LoggedInUser />);
    const user = await openMenu();
    expect(screen.getByRole("menuitem", { name: "Billing" })).toBeInTheDocument();
    await user.click(screen.getByRole("menuitem", { name: "Dashboard" }));
    expect(assign).toHaveBeenCalledWith("/dashboard");
  });

  it.each([
    ["Settings", "/settings"],
    ["Billing", "/billing"],
    ["Users", "/settings/users"],
    ["Troubleshooting", "/troubleshooting"],
    ["Docs", "/docs"],
  ])("sends %s to %s", async (name, href) => {
    signIn();
    render(() => <LoggedInUser />);
    const user = await openMenu();
    await user.click(screen.getByRole("menuitem", { name }));
    expect(assign).toHaveBeenCalledWith(href);
  });

  it("hides billing and users from members who aren't admins", async () => {
    signIn(testSession({ is_admin: false }));
    render(() => <LoggedInUser />);
    await openMenu();
    expect(screen.getByRole("menuitem", { name: "Settings" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Billing" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Users" })).not.toBeInTheDocument();
  });

  it("hides the dashboard link on the dashboard", async () => {
    stubLocation("/dashboard/pages");
    signIn();
    render(() => <LoggedInUser />);
    await openMenu();
    expect(screen.queryByRole("menuitem", { name: "Dashboard" })).not.toBeInTheDocument();
  });

  it("selects a project in the shared selection", async () => {
    signIn(twoProjects);
    render(() => <LoggedInUser />);
    const user = await openMenu();
    await user.click(screen.getByRole("menuitemradio", { name: "beta" }));
    expect(selectedProjectId()).toBe("2");
  });

  it("hands the project change to the parent when it owns the selection", async () => {
    signIn(twoProjects);
    const onProjectChange = vi.fn();
    render(() => (
      <LoggedInUser
        projects={[
          { label: "one", value: "1", timezone: "UTC" },
          { label: "two", value: "2", timezone: "UTC" },
        ]}
        projectId="1"
        onProjectChange={onProjectChange}
      />
    ));
    const user = await openMenu();
    await user.click(screen.getByRole("menuitemradio", { name: "one" }));
    expect(onProjectChange).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("button", { name: "Account menu" }));
    await user.click(await screen.findByRole("menuitemradio", { name: "two" }));
    expect(onProjectChange).toHaveBeenCalledWith("2");
    expect(selectedProjectId()).toBe("");
  });

  it("logs out", async () => {
    let loggedOut = false;
    server.use(
      http.post(`${API}/v2/public/auth/logout`, () => {
        loggedOut = true;
        return new HttpResponse(null);
      }),
    );
    signIn();
    render(() => <LoggedInUser />);
    const user = await openMenu();
    await user.click(screen.getByRole("menuitem", { name: "Log out" }));
    await vi.waitFor(() => expect(loggedOut).toBe(true));
    await vi.waitFor(() =>
      expect(screen.queryByRole("button", { name: "Account menu" })).not.toBeInTheDocument(),
    );
  });
});

describe("AuthButton", () => {
  it("offers the login dialog while signed out", async () => {
    render(() => <AuthButton triggerLabel="Sign in" />);
    expect(await screen.findByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });

  it("shows the account menu once signed in", async () => {
    render(() => {
      useAuthState();
      return <AuthButton />;
    });
    signIn();
    expect(await screen.findByRole("button", { name: "Account menu" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Log in" })).not.toBeInTheDocument();
  });
});
