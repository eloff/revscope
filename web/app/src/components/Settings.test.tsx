import { selectedProjectId, setSelectedProjectId } from "@revscope/web-shared/projectSelection";
import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { signIn, signOut } from "../../../shared/test/auth";
import { testProject, testSession } from "../../../shared/test/fixtures";
import { stubLocation } from "../../../shared/test/location";
import type { SettingsResponse } from "../lib/settingsApi";
import Settings from "./Settings";

const session = testSession({
  projects: { alpha_com: testProject("1", "UTC"), beta_com: testProject("2", "Europe/Paris") },
});

function settings(overrides: Partial<SettingsResponse> = {}): SettingsResponse {
  return {
    timezone: "UTC",
    payment_success_path: "/thanks",
    custom_domain: null,
    custom_domain_included: true,
    integrations: [],
    ...overrides,
  };
}

const stripe = {
  integration_type: "stripe" as const,
  external_account_id: null,
  api_key_masked: "sk_live_…1234",
  secret_masked: null,
  last_synced_at: null,
  last_sync_error: null,
};

beforeEach(() => {
  stubLocation("/settings");
  setSelectedProjectId("1");
  mockApi("get", "/v2/session/projects/:id/ga-imports", {
    oauth_available: false,
    oldest_allowed_date: null,
    latest_allowed_date: "2026-11-03",
    default_start_date: "2020-10-14",
    default_end_date: "2026-11-03",
    first_tracked_date: null,
    include_in_reports: true,
    imports: [],
  });
});

afterEach(async () => {
  vi.unstubAllGlobals();
  setSelectedProjectId("");
  await signOut();
});

async function renderSettings(response = settings(), projectId = "1") {
  signIn(session);
  const loads = mockApi("get", `/v2/session/projects/${projectId}/settings`, response);
  render(() => <Settings />);
  await screen.findByRole("heading", { name: /settings$/ });
  await screen.findByLabelText(/Payment success path/);
  return { loads, user: userEvent.setup() };
}

describe("Settings", () => {
  it("clears a saved custom domain by sending null", async () => {
    const { user } = await renderSettings(settings({ custom_domain: "assets.example.com" }));
    const saves = mockApi("post", "/v2/session/projects/1/settings", settings());
    await user.clear(screen.getByLabelText(/Custom domain/));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
    const body = saves[0]?.body as { custom_domain: unknown };
    expect(body.custom_domain).toBeNull();
  });

  it("offers a custom domain when the subscription includes one", async () => {
    await renderSettings();
    expect(screen.getByLabelText(/Custom domain/)).toBeEnabled();
    expect(screen.getByText(/First-party hostname for the tracker/)).toBeInTheDocument();
  });

  it("disables the custom domain when the subscription doesn't include one", async () => {
    await renderSettings(settings({ custom_domain_included: false }));
    expect(screen.getByLabelText(/Custom domain/)).toBeDisabled();
    expect(screen.getByRole("link", { name: "Upgrade your plan" })).toHaveAttribute(
      "href",
      "/billing",
    );
  });

  it("lets an account without the feature clear a domain it kept", async () => {
    const { user } = await renderSettings(
      settings({ custom_domain: "assets.example.com", custom_domain_included: false }),
    );
    expect(screen.getByText(/Your plan doesn't include this domain/)).toBeInTheDocument();
    const saves = mockApi(
      "post",
      "/v2/session/projects/1/settings",
      settings({ custom_domain_included: false }),
    );
    const field = screen.getByLabelText(/Custom domain/);
    await user.clear(field);
    expect(field).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
    const body = saves[0]?.body as { custom_domain: unknown };
    expect(body.custom_domain).toBeNull();
    expect(screen.getByLabelText(/Custom domain/)).toBeDisabled();
  });

  it("asks a signed-out visitor to log in", async () => {
    render(() => <Settings />);
    expect(await screen.findByText("Log in to manage your settings.")).toBeInTheDocument();
  });

  it("loads the selected project's settings", async () => {
    const { loads } = await renderSettings();
    expect(screen.getByRole("heading", { name: "alpha_com settings" })).toBeInTheDocument();
    expect(screen.getByLabelText(/Payment success path/)).toHaveValue("/thanks");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(loads[0]?.credentials).toBe("include");
  });

  it("shows the Google Analytics import to admins only", async () => {
    await renderSettings();
    expect(
      await screen.findByRole("heading", { name: "Import from Google Analytics" }),
    ).toBeInTheDocument();
  });

  it("lets members read settings but not save or delete", async () => {
    signIn({ ...session, is_admin: false });
    mockApi("get", "/v2/session/projects/1/settings", settings());
    render(() => <Settings />);
    expect(
      await screen.findByText("Only account admins can change these settings."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Import from Google Analytics" })).toBeNull();
    expect(screen.getByLabelText(/Payment success path/)).toHaveValue("/thanks");
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(screen.queryByText("Danger zone")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Manage users" })).not.toBeInTheDocument();
  });

  it("links admins to user management", async () => {
    await renderSettings();
    expect(screen.getByRole("link", { name: "Manage users" })).toHaveAttribute(
      "href",
      "/settings/users",
    );
  });

  it("shows a failure to load", async () => {
    signIn(session);
    mockApi("get", "/v2/session/projects/1/settings", { error: "Database unavailable." }, 503);
    render(() => <Settings />);
    expect(await screen.findByText("Database unavailable.")).toBeInTheDocument();
  });

  it("says so when the account has no projects", async () => {
    signIn(testSession({ projects: {} }));
    render(() => <Settings />);
    expect(await screen.findByText("This account has no projects yet.")).toBeInTheDocument();
  });

  it("saves edits, leaving untouched fields and secrets alone", async () => {
    const { user } = await renderSettings(settings({ integrations: [stripe] }));
    const saves = mockApi(
      "post",
      "/v2/session/projects/1/settings",
      settings({ payment_success_path: "/done" }),
    );
    const path = screen.getByLabelText(/Payment success path/);
    await user.clear(path);
    await user.type(path, " /done ");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
    const body = saves[0]?.body as {
      payment_success_path: string;
      custom_domain?: string | null;
      integrations: { integration_type: string; api_key: unknown }[];
    };
    expect(body.payment_success_path).toBe("/done");
    expect(body).not.toHaveProperty("custom_domain");
    expect(
      body.integrations.find((entry) => entry.integration_type === "stripe")?.api_key,
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("replaces a stored secret only when asked", async () => {
    const { user } = await renderSettings(settings({ integrations: [stripe] }));
    expect(screen.getByText("sk_live_…1234")).toBeInTheDocument();
    expect(screen.getAllByText("Connected").length).toBeGreaterThan(0);
    expect(screen.getByText("Waiting for first sync…")).toBeInTheDocument();

    await user.click(screen.getByText("Replace"));
    await user.type(screen.getByLabelText("Secret key"), "sk_live_new");
    const saves = mockApi(
      "post",
      "/v2/session/projects/1/settings",
      settings({ integrations: [stripe] }),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Settings saved.");
    const body = saves[0]?.body as {
      integrations: { integration_type: string; api_key: unknown }[];
    };
    expect(body.integrations.find((entry) => entry.integration_type === "stripe")?.api_key).toBe(
      "sk_live_new",
    );
  });

  it("fills Search Console's email and key from a pasted key file", async () => {
    const { user } = await renderSettings();
    const tab = screen.getByRole("tab", { name: "Search Console" });
    await user.click(tab);
    const panelElement = document.getElementById(tab.getAttribute("aria-controls") ?? "");
    if (!panelElement) {
      throw new Error("the Search Console tab has no panel");
    }
    const panel = within(panelElement);
    expect(panel.getByText("How to create a service account")).toBeVisible();

    const keyFile = panel.getByLabelText("Service account JSON key");
    await user.click(keyFile);
    await user.paste("{ not json");
    expect(screen.getByText(/isn't a service account key/)).toBeInTheDocument();

    await user.clear(keyFile);
    await user.click(keyFile);
    await user.paste(
      JSON.stringify({
        client_email: "rs@example.iam.gserviceaccount.com",
        private_key: "-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\n",
      }),
    );
    expect(keyFile).toHaveValue("");
    expect(screen.queryByText(/isn't a service account key/)).not.toBeInTheDocument();
    expect(screen.getByLabelText("Service account email")).toHaveValue(
      "rs@example.iam.gserviceaccount.com",
    );

    await user.type(screen.getByLabelText("Site / property URL"), "sc-domain:example.com");
    const saves = mockApi("post", "/v2/session/projects/1/settings", settings());
    await user.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Settings saved.");
    const body = saves[0]?.body as {
      integrations: {
        integration_type: string;
        external_account_id: unknown;
        api_key: unknown;
        secret: unknown;
      }[];
    };
    expect(
      body.integrations.find((entry) => entry.integration_type === "google_search_console"),
    ).toMatchObject({
      external_account_id: "sc-domain:example.com",
      api_key: "rs@example.iam.gserviceaccount.com",
      secret: "-----BEGIN PRIVATE KEY-----\\nABC\\n-----END PRIVATE KEY-----",
    });
  });

  it("cancels a secret replacement", async () => {
    const { user } = await renderSettings(settings({ integrations: [stripe] }));
    await user.click(screen.getByText("Replace"));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("sk_live_…1234")).toBeInTheDocument();
  });

  it("reports the last sync result", async () => {
    await renderSettings(
      settings({
        integrations: [
          {
            ...stripe,
            last_synced_at: new Date(Date.now() - 2 * 3600_000).toISOString(),
            last_finished_sync_error: "Key revoked",
          },
        ],
      }),
    );
    expect(screen.getByText(/Last sync failed: Key revoked/)).toHaveTextContent("2 hours ago");
  });

  it("shows a save error", async () => {
    const { user } = await renderSettings();
    mockApi("post", "/v2/session/projects/1/settings", { error: "Custom domain is invalid." }, 400);
    await user.type(screen.getByLabelText(/Custom domain/), "nope");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Custom domain is invalid.")).toBeInTheDocument();
  });

  describe("unsaved changes", () => {
    it("holds back a link click until changes are discarded", async () => {
      const { user } = await renderSettings();
      const link = document.createElement("a");
      link.href = "/dashboard";
      link.textContent = "Dashboard";
      document.body.append(link);

      await user.type(screen.getByLabelText(/Custom domain/), "x");
      await user.click(link);
      const dialog = await screen.findByRole("dialog");
      expect(within(dialog).getByText("Unsaved changes")).toBeInTheDocument();
      await user.click(within(dialog).getByRole("button", { name: "Discard changes" }));
      expect(new URL(window.location.href).pathname).toBe("/dashboard");
      link.remove();
    });

    it("keeps the current project when a switch is cancelled", async () => {
      const { user, loads } = await renderSettings();
      await user.type(screen.getByLabelText(/Custom domain/), "x");
      setSelectedProjectId("2");
      const dialog = await screen.findByRole("dialog");
      await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(selectedProjectId()).toBe("1");
      expect(loads).toHaveLength(1);
      expect(screen.getByLabelText(/Custom domain/)).toHaveValue("x");
    });

    it("saves, then switches project", async () => {
      const { user } = await renderSettings();
      const saves = mockApi("post", "/v2/session/projects/1/settings", settings());
      const betaLoads = mockApi(
        "get",
        "/v2/session/projects/2/settings",
        settings({ timezone: "Europe/Paris" }),
      );
      await user.type(screen.getByLabelText(/Custom domain/), "x");
      setSelectedProjectId("2");
      const dialog = await screen.findByRole("dialog");
      await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
      expect(await screen.findByRole("heading", { name: "beta_com settings" })).toBeInTheDocument();
      expect(saves).toHaveLength(1);
      expect(betaLoads).toHaveLength(1);
    });

    it("switches straight away when nothing changed", async () => {
      await renderSettings();
      mockApi("get", "/v2/session/projects/2/settings", settings());
      setSelectedProjectId("2");
      expect(await screen.findByRole("heading", { name: "beta_com settings" })).toBeInTheDocument();
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });
});
