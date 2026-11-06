import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../../shared/test/api";
import { signIn, signOut } from "../../../../shared/test/auth";
import { testSession } from "../../../../shared/test/fixtures";
import { stubLocation } from "../../../../shared/test/location";
import DownloadDbButton from "./DownloadDbButton";

const TOKEN = "/v2/session/projects/42/download-token";

afterEach(() => {
  signOut();
  vi.unstubAllGlobals();
});

async function openDialog() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Download project database" }));
  return { user, dialog: await screen.findByRole("dialog") };
}

describe("DownloadDbButton", () => {
  it("hides for a private project without a session", () => {
    render(() => <DownloadDbButton projectId="42" />);
    expect(screen.queryByRole("button", { name: "Download project database" })).toBeNull();
  });

  it("asks a visitor who isn't signed in to sign in, then returns them to the demo", async () => {
    const assign = stubLocation("/demo", "?tab=overview");
    const calls = mockApi("post", "/v2/session/projects/1/download-token", {});
    mockApi("post", "/v2/public/auth/login", testSession());
    // Rendered alone, as on /demo, which mounts no other login dialog.
    render(() => <DownloadDbButton projectId="1" />);
    const { user, dialog } = await openDialog();
    expect(within(dialog).getByText("Sign in to download the demo database.")).toBeInTheDocument();
    expect(within(dialog).queryByRole("link", { name: "OK" })).toBeNull();

    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));
    const login = await screen.findByRole("dialog", { name: "Log in" });
    await user.type(within(login).getByLabelText("Email"), "ada@example.com");
    await user.type(within(login).getByLabelText("Password"), "correctHorse9!");
    await user.click(within(login).getByRole("button", { name: "Log in" }));
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/demo?tab=overview"));
    expect(calls).toHaveLength(0);
  });

  it("prompts to sign in again when the API rejects a cached session", async () => {
    signIn();
    mockApi("post", "/v2/session/projects/1/download-token", { error: "invalid session" }, 401);
    mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    render(() => <DownloadDbButton projectId="1" />);
    const { dialog } = await openDialog();
    expect(
      await within(dialog).findByText("Sign in to download the demo database."),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Sign in" })).toBeEnabled();
  });

  it("downloads the demo database with a token once signed in", async () => {
    signIn();
    const calls = mockApi("post", "/v2/session/projects/1/download-token", {
      token: "t-demo",
      size_bytes: 1024,
      expires_at: 1,
    });
    render(() => <DownloadDbButton projectId="1" />);
    const { dialog } = await openDialog();
    expect(await within(dialog).findByText("Download size: 1.0 kb")).toBeInTheDocument();
    const link = within(dialog).getByRole("link", { name: "OK" });
    expect(link).toHaveAttribute(
      "href",
      "http://localhost:4320/v2/download/projects/1?token=t-demo",
    );
    expect(link).toHaveAttribute("download", "revscope-project-1.db");
    expect(calls).toHaveLength(1);
  });

  it("mints a token only once the dialog opens", async () => {
    signIn();
    const calls = mockApi("post", TOKEN, { token: "t-9", size_bytes: 356_725_555, expires_at: 1 });
    render(() => <DownloadDbButton projectId="42" />);
    expect(calls).toHaveLength(0);

    const { dialog } = await openDialog();
    expect(await within(dialog).findByText("Download size: 340.2 mb")).toBeInTheDocument();
    expect(within(dialog).getByRole("link", { name: "OK" })).toHaveAttribute(
      "href",
      "http://localhost:4320/v2/download/projects/42?token=t-9",
    );
    expect(calls).toHaveLength(1);
  });

  it("shows why the download can't be prepared", async () => {
    signIn();
    mockApi("post", TOKEN, { error: "Downloads are disabled on this plan." }, 403);
    render(() => <DownloadDbButton projectId="42" />);
    const { dialog } = await openDialog();
    expect(
      await within(dialog).findByText("Downloads are disabled on this plan."),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "OK" })).toBeDisabled();
    expect(within(dialog).queryByText(/Download size/)).toBeNull();
  });
});
