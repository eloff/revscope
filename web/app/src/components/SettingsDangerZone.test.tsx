import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { testProject, testSession } from "../../../shared/test/fixtures";
import SettingsDangerZone from "./SettingsDangerZone";

const REVOKE = "/v2/session/mcp/revoke";
const DELETE_PATH = "/v2/session/projects/42";

function renderZone({ projectCount = 2, onProjectDeleted = vi.fn() } = {}) {
  render(() => (
    <SettingsDangerZone
      session={testSession()}
      project={{ id: "42", name: "example_com" }}
      projectCount={projectCount}
      onProjectDeleted={onProjectDeleted}
    />
  ));
  return { onProjectDeleted };
}

async function confirmRevoke() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Revoke all MCP tokens" }));
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByRole("button", { name: "Revoke all MCP tokens" }));
  return { user, dialog };
}

describe("SettingsDangerZone", () => {
  it("asks before revoking, and cancelling sends nothing", async () => {
    const calls = mockApi("post", REVOKE, { revoked: 1 });
    renderZone();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Revoke all MCP tokens" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls).toHaveLength(0);
  });

  it.each([
    [0, "No MCP tokens to revoke."],
    [1, "Revoked 1 MCP token."],
    [4, "Revoked 4 MCP tokens."],
  ])("reports %d revoked tokens", async (revoked, message) => {
    const calls = mockApi("post", REVOKE, { revoked });
    renderZone();
    await confirmRevoke();
    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(calls[0]?.credentials).toBe("include");
  });

  it("shows the server's error", async () => {
    mockApi("post", REVOKE, { error: "Session expired." }, 401);
    renderZone();
    await confirmRevoke();
    expect(await screen.findByText("Session expired.")).toBeInTheDocument();
  });

  it("deletes the project only once its name is typed", async () => {
    const calls = mockApi("delete", DELETE_PATH, { deleted: "42" });
    const me = mockApi(
      "get",
      "/v2/session/me",
      testSession({ plan: "growth", projects: { other_com: testProject("7") } }),
    );
    const { onProjectDeleted } = renderZone();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Delete project" }));
    const dialog = await screen.findByRole("dialog");
    const confirm = within(dialog).getByRole("button", { name: "Delete project" });
    await user.type(within(dialog).getByRole("textbox"), "example");
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox"), "_com");
    await user.click(confirm);
    await vi.waitFor(() => expect(onProjectDeleted).toHaveBeenCalledWith("42"));
    expect(calls).toHaveLength(1);
    expect(calls[0]?.body).toEqual({ confirm_name: "example_com" });
    expect(calls[0]?.credentials).toBe("include");
    // The project list everywhere comes from /me, so it is reloaded.
    expect(me).toHaveLength(1);
  });

  it("shows the server's refusal and keeps the dialog open", async () => {
    mockApi("delete", DELETE_PATH, { error: "Type the project name exactly to confirm." }, 400);
    const { onProjectDeleted } = renderZone();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Delete project" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "example_com");
    await user.click(within(dialog).getByRole("button", { name: "Delete project" }));
    expect(
      await within(dialog).findByText("Type the project name exactly to confirm."),
    ).toBeInTheDocument();
    expect(onProjectDeleted).not.toHaveBeenCalled();
  });

  it("won't delete the account's only project", () => {
    renderZone({ projectCount: 1 });
    expect(screen.getByRole("button", { name: "Delete project" })).toBeDisabled();
    expect(screen.getByText(/only project/)).toBeInTheDocument();
  });
});
