import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../../shared/test/api";
import { signIn, signOut } from "../../../../shared/test/auth";
import { testProject, testSession } from "../../../../shared/test/fixtures";
import type { AccountUser } from "../../lib/usersApi";
import ManageUsers from "./ManageUsers";

const USERS = "/v2/session/users";
const INVITE = "/v2/session/users/invite";
const UPDATE = "/v2/session/users/update";
const REMOVE = "/v2/session/users/remove";

const admin = testSession({
  user_id: "u-ada",
  projects: { alpha: testProject("1"), beta: testProject("2") },
});

const ada: AccountUser = {
  user_id: "u-ada",
  name: "Ada",
  email: "ada@example.com",
  pending: false,
  last_login_at: 1,
  is_admin: true,
  project_ids: null,
};
const bo: AccountUser = {
  user_id: "u-bo",
  name: "Bo",
  email: "bo@example.com",
  pending: true,
  last_login_at: null,
  is_admin: false,
  project_ids: [2],
};

beforeEach(() => signIn(admin));
afterEach(async () => {
  vi.restoreAllMocks();
  await signOut();
});

function row(name: string) {
  return within(screen.getByRole("listitem", { name }));
}

async function fillInvite(name: string, email: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Name"), name);
  await user.type(screen.getByLabelText("Email"), email);
  return user;
}

function submitInvite() {
  const form = screen.getByRole("button", { name: "Invite" }).closest("form");
  if (form) fireEvent.submit(form);
}

describe("ManageUsers", () => {
  it("tells members only admins can manage users, without loading the list", async () => {
    signIn(testSession({ is_admin: false }));
    render(() => <ManageUsers />);
    expect(await screen.findByText("Only account admins can manage users.")).toBeInTheDocument();
  });

  it("lists users with their role, invite state and projects", async () => {
    mockApi("get", USERS, { users: [ada, bo] });
    render(() => <ManageUsers />);
    await screen.findByText("bo@example.com");
    expect(row("Ada").getByText("Admin")).toBeInTheDocument();
    expect(row("Ada").getByText("You")).toBeInTheDocument();
    expect(row("Ada").getByText("All projects")).toBeInTheDocument();
    // Admins can't change themselves.
    expect(row("Ada").queryByRole("button", { name: "Edit access" })).not.toBeInTheDocument();
    expect(row("Bo").getByText("Invited")).toBeInTheDocument();
    expect(row("Bo").getByText("beta")).toBeInTheDocument();
    expect(row("Bo").getByRole("button", { name: "Remove" })).toBeInTheDocument();
  });

  it("shows a failure to load users", async () => {
    mockApi("get", USERS, { error: "Only an account admin can do this." }, 403);
    render(() => <ManageUsers />);
    expect(await screen.findByText("Only an account admin can do this.")).toBeInTheDocument();
  });

  it("invites a member to chosen projects and reloads the list", async () => {
    const lists = mockApi("get", USERS, { users: [ada] });
    const invites = mockApi("post", INVITE, { invite_url: "https://x/invite/1", emailed: false });
    render(() => <ManageUsers />);
    await screen.findByText("ada@example.com");

    const user = await fillInvite(" Cy ", "cy@example.com ");
    await user.click(screen.getByLabelText("Only these projects"));
    await user.click(screen.getByLabelText("beta"));
    submitInvite();

    expect(
      await screen.findByText("Send this link to cy@example.com. It expires in 7 days:"),
    ).toBeInTheDocument();
    expect(invites[0]?.body).toEqual({
      name: "Cy",
      email: "cy@example.com",
      is_admin: false,
      project_ids: [2],
    });
    expect(screen.getByLabelText("Invite link")).toHaveValue("https://x/invite/1");
    await vi.waitFor(() => expect(lists).toHaveLength(2));
  });

  it("invites an admin to every project", async () => {
    mockApi("get", USERS, { users: [ada] });
    const invites = mockApi("post", INVITE, { invite_url: "https://x/invite/1", emailed: true });
    render(() => <ManageUsers />);
    await screen.findByText("ada@example.com");
    const user = await fillInvite("Cy", "cy@example.com");
    await user.click(screen.getByLabelText(/^Admin/));
    expect(screen.queryByLabelText("Only these projects")).not.toBeInTheDocument();
    submitInvite();
    expect(await screen.findByText(/Emailed an invite to cy@example.com/)).toBeInTheDocument();
    expect(invites[0]?.body).toEqual({
      name: "Cy",
      email: "cy@example.com",
      is_admin: true,
      project_ids: null,
    });
  });

  it("refuses to invite a member to no projects", async () => {
    mockApi("get", USERS, { users: [ada] });
    const invites = mockApi("post", INVITE, { invite_url: "https://x/invite/1", emailed: false });
    render(() => <ManageUsers />);
    await screen.findByText("ada@example.com");
    const user = await fillInvite("Cy", "cy@example.com");
    await user.click(screen.getByLabelText("Only these projects"));
    submitInvite();
    expect(
      await screen.findByText("Choose at least one project, or give access to all of them."),
    ).toBeInTheDocument();
    expect(invites).toHaveLength(0);
  });

  it("copies the invite link", async () => {
    mockApi("get", USERS, { users: [] });
    mockApi("post", INVITE, { invite_url: "https://x/invite/1", emailed: false });
    render(() => <ManageUsers />);
    const user = await fillInvite("Cy", "cy@example.com");
    submitInvite();
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    await user.click(await screen.findByRole("button", { name: "Copy" }));
    expect(writeText).toHaveBeenCalledWith("https://x/invite/1");
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("changes a member's access and reloads the list", async () => {
    const lists = mockApi("get", USERS, { users: [ada, bo] });
    const updates = mockApi("post", UPDATE, { ok: true });
    render(() => <ManageUsers />);
    await screen.findByText("bo@example.com");
    const user = userEvent.setup();
    await user.click(row("Bo").getByRole("button", { name: "Edit access" }));
    // The editor starts from Bo's current access.
    expect(row("Bo").getByLabelText("beta")).toBeChecked();
    expect(row("Bo").getByLabelText("alpha")).not.toBeChecked();
    await user.click(row("Bo").getByLabelText("alpha"));
    await user.click(row("Bo").getByRole("button", { name: "Save access" }));
    await vi.waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]?.body).toEqual({ user_id: "u-bo", is_admin: false, project_ids: [1, 2] });
    await vi.waitFor(() => expect(lists).toHaveLength(2));
  });

  it("shows why an access change failed", async () => {
    mockApi("get", USERS, { users: [ada, bo] });
    mockApi("post", UPDATE, { error: "Only an account admin can do this." }, 403);
    render(() => <ManageUsers />);
    await screen.findByText("bo@example.com");
    const user = userEvent.setup();
    await user.click(row("Bo").getByRole("button", { name: "Edit access" }));
    await user.click(row("Bo").getByLabelText(/^Admin/));
    await user.click(row("Bo").getByRole("button", { name: "Save access" }));
    expect(await row("Bo").findByText("Only an account admin can do this.")).toBeInTheDocument();
  });

  it("removes a user after confirming", async () => {
    const lists = mockApi("get", USERS, { users: [ada, bo] });
    const removals = mockApi("post", REMOVE, { ok: true });
    render(() => <ManageUsers />);
    await screen.findByText("bo@example.com");
    const user = userEvent.setup();
    await user.click(row("Bo").getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Their invite link stops working.")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await vi.waitFor(() => expect(removals).toHaveLength(1));
    expect(removals[0]?.body).toEqual({ user_id: "u-bo" });
    await vi.waitFor(() => expect(lists).toHaveLength(2));
  });
});
