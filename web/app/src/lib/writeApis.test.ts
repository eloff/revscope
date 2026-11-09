import { describe, expect, it } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { testSession } from "../../../shared/test/fixtures";
import {
  deleteCampaign,
  fetchCampaigns,
  previewCampaign,
  saveCampaign,
  saveCampaignSpend,
} from "./campaignsApi";
import {
  createDownloadToken,
  formatFileSize,
  projectDatabaseDownloadUrl,
  projectDatabaseFileName,
} from "./downloadDbApi";
import {
  deleteFunnel,
  fetchFunnelData,
  fetchFunnelDataBootstrap,
  fetchFunnelDataPreview,
  fetchFunnelOptions,
  fetchFunnels,
  type SaveFunnelRequest,
  saveFunnel,
} from "./funnelsApi";
import { EMPTY_SEGMENT_FILTERS } from "./segmentFilters";
import {
  fetchSettings,
  revokeMcpTokens,
  type SaveSettingsPayload,
  saveSettings,
} from "./settingsApi";
import { fetchAccountUsers, inviteUser, removeUser, updateUserAccess } from "./usersApi";

const session = testSession();
const rules = [{ param_key: "utm_campaign", param_value: "spring" }];

describe("settings client", () => {
  const path = "/v2/session/projects/42/settings";
  const payload: SaveSettingsPayload = {
    timezone: "UTC",
    payment_success_path: "/thanks",
    custom_domain: null,
    integrations: [],
  };

  it("loads settings with the session cookie", async () => {
    const calls = mockApi("get", path, { timezone: "UTC" });
    await expect(fetchSettings(session, "42")).resolves.toEqual({ timezone: "UTC" });
    expect(calls[0]?.credentials).toBe("include");
  });

  it("posts the full settings document as JSON", async () => {
    const calls = mockApi("post", path, { timezone: "UTC" });
    await saveSettings(session, "42", payload);
    expect(calls[0]?.body).toEqual(payload);
    expect(calls[0]?.headers.get("content-type")).toBe("application/json");
  });

  it.each([
    ["load", () => fetchSettings(session, "42"), "get", "Unable to load settings."],
    ["save", () => saveSettings(session, "42", payload), "post", "Unable to save settings."],
  ] as const)("reports a failed %s", async (_label, call, method, fallback) => {
    mockApi(method, path, {}, 500);
    await expect(call()).rejects.toThrow(fallback);
    mockApi(method, path, { error: "Bad timezone." }, 400);
    await expect(call()).rejects.toThrow("Bad timezone.");
  });

  it("revokes MCP tokens", async () => {
    const calls = mockApi("post", "/v2/session/mcp/revoke", { revoked: 3 });
    await expect(revokeMcpTokens(session)).resolves.toEqual({ revoked: 3 });
    expect(calls).toHaveLength(1);
    mockApi("post", "/v2/session/mcp/revoke", {}, 500);
    await expect(revokeMcpTokens(session)).rejects.toThrow("Unable to revoke MCP tokens.");
  });
});

describe("users client", () => {
  const access = { is_admin: false, project_ids: [42] };

  it("lists account users and treats a missing list as an error", async () => {
    const users = [
      {
        user_id: "u1",
        name: "Ada",
        email: "a@x.io",
        pending: false,
        last_login_at: null,
        is_admin: true,
        project_ids: null,
      },
    ];
    mockApi("get", "/v2/session/users", { users, invites_included: true });
    await expect(fetchAccountUsers(session)).resolves.toEqual({ users, invites_included: true });
    mockApi("get", "/v2/session/users", {});
    await expect(fetchAccountUsers(session)).rejects.toThrow("Unable to load users.");
  });

  it("invites a user with their access", async () => {
    const calls = mockApi("post", "/v2/session/users/invite", {
      invite_url: "https://x/invite",
      emailed: false,
    });
    const invite = { name: "Bo", email: "bo@x.io", ...access };
    await expect(inviteUser(session, invite)).resolves.toEqual({
      invite_url: "https://x/invite",
      emailed: false,
    });
    expect(calls[0]?.body).toEqual(invite);
    mockApi("post", "/v2/session/users/invite", { error: "Already a member." }, 409);
    await expect(inviteUser(session, invite)).rejects.toThrow("Already a member.");
  });

  it("changes a user's access", async () => {
    const calls = mockApi("post", "/v2/session/users/update", { ok: true });
    await updateUserAccess(session, "u2", access);
    expect(calls[0]?.body).toEqual({ user_id: "u2", ...access });
    mockApi(
      "post",
      "/v2/session/users/update",
      { error: "Only an account admin can do this." },
      403,
    );
    await expect(updateUserAccess(session, "u2", access)).rejects.toThrow(
      "Only an account admin can do this.",
    );
  });

  it("removes a user", async () => {
    const calls = mockApi("post", "/v2/session/users/remove", { ok: true });
    await removeUser(session, "u2");
    expect(calls[0]?.body).toEqual({ user_id: "u2" });
    mockApi("post", "/v2/session/users/remove", {}, 500);
    await expect(removeUser(session, "u2")).rejects.toThrow("Unable to remove the user.");
  });
});

describe("campaigns client", () => {
  it("loads campaigns for the range and segments, signed in or not", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/campaigns", { campaigns: [] });
    await fetchCampaigns(session, "42", 10, 20, {
      ...EMPTY_SEGMENT_FILTERS,
      countries: [{ code: "DE", name: "Germany" }],
    });
    await fetchCampaigns(undefined, "42", 10, 20, EMPTY_SEGMENT_FILTERS);
    expect(queryOf(calls[0])).toEqual({ start: "10", end: "20", country: "DE" });
    expect(calls.map((call) => call.credentials)).toEqual(["include", "omit"]);
  });

  it("previews rules with a POST carrying the project id", async () => {
    const calls = mockApi("post", "/v2/read/projects/42/campaign-preview", { direct: 1 });
    await previewCampaign(session, "42", 10, 20, EMPTY_SEGMENT_FILTERS, rules);
    await previewCampaign(undefined, "42", 10, 20, EMPTY_SEGMENT_FILTERS, rules);
    expect(calls[0]?.body).toEqual({ rules });
    expect(queryOf(calls[0])).toEqual({ start: "10", end: "20" });
    expect(calls[1]?.credentials).toBe("omit");
  });

  it("saves a campaign under the project", async () => {
    const calls = mockApi("post", "/v2/session/projects/42/campaigns", { id: 5, assigned: 2 });
    await expect(saveCampaign(session, "42", { name: "Spring", rules })).resolves.toEqual({
      id: 5,
      assigned: 2,
    });
    expect(calls[0]?.body).toEqual({ name: "Spring", rules });
  });

  it("replaces a campaign's spend", async () => {
    const entries = [{ kind: "once" as const, start_date: "2026-01-01", amount: 5000 }];
    const calls = mockApi("post", "/v2/session/projects/42/campaign-spend", {});
    await saveCampaignSpend(session, "42", 5, entries);
    expect(calls[0]?.body).toEqual({ campaign_id: 5, entries });
  });

  it("deletes by id", async () => {
    const calls = mockApi("delete", "/v2/session/projects/42/campaigns", {});
    await deleteCampaign(session, "42", 5);
    expect(queryOf(calls[0])).toEqual({ id: "5" });
  });

  it.each([
    [
      "load",
      "get",
      "campaigns",
      () => fetchCampaigns(session, "42", 1, 2, EMPTY_SEGMENT_FILTERS),
      "Unable to load campaigns.",
    ],
    [
      "save",
      "post",
      "campaigns",
      () => saveCampaign(session, "42", { name: "x", rules }),
      "Unable to save this campaign.",
    ],
    [
      "spend",
      "post",
      "campaign-spend",
      () => saveCampaignSpend(session, "42", 1, []),
      "Unable to save spend.",
    ],
    [
      "delete",
      "delete",
      "campaigns",
      () => deleteCampaign(session, "42", 1),
      "Unable to delete this campaign.",
    ],
  ] as const)("reports a failed %s", async (_label, method, resource, call, fallback) => {
    const policy = method === "get" ? "read" : "session";
    mockApi(method, `/v2/${policy}/projects/42/${resource}`, {}, 500);
    await expect(call()).rejects.toThrow(fallback);
  });
});

describe("funnels client", () => {
  const range = { start: 100, end: 200, previous_start: 0, previous_end: 100 };
  const steps = [{ kind: "pageview", path_id: 1 }] as unknown as SaveFunnelRequest["steps"];

  it("lists funnels and options, signed in or not", async () => {
    const list = mockApi("get", "/v2/read/projects/42/funnels", { funnels: [] });
    const options = mockApi("get", "/v2/read/projects/42/funnel-options", { pages: [] });
    await fetchFunnels(session, "42");
    await fetchFunnels(undefined, "42");
    await fetchFunnelOptions(session, "42");
    await fetchFunnelOptions(undefined, "42");
    expect(list.map((call) => call.credentials)).toEqual(["include", "omit"]);
    expect(options.map((call) => call.credentials)).toEqual(["include", "omit"]);
  });

  it("loads one funnel's data for both periods", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/funnel-data", {});
    await fetchFunnelData(session, "42", 7, 100, 200, 0, 100);
    await fetchFunnelData(undefined, "42", 7, 100, 200, 0, 100);
    expect(queryOf(calls[0])).toEqual({
      funnel_id: "7",
      start: "100",
      end: "200",
      previous_start: "0",
      previous_end: "100",
    });
    expect(calls[1]?.credentials).toBe("omit");
  });

  it("bootstraps without a funnel id", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/funnel-data", {});
    await fetchFunnelDataBootstrap(session, "42", 100, 200, 0, 100);
    expect(calls[0]?.url.searchParams.has("funnel_id")).toBe(false);
  });

  it("previews unsaved steps", async () => {
    const calls = mockApi("post", "/v2/read/projects/42/funnel-preview", {});
    await fetchFunnelDataPreview(session, "42", { steps, ...range });
    await fetchFunnelDataPreview(undefined, "42", { steps, ...range });
    expect(calls[0]?.body).toEqual({ steps, ...range });
    expect(calls[1]?.credentials).toBe("omit");
  });

  it("saves and deletes through the session policy", async () => {
    const saved = mockApi("post", "/v2/session/projects/42/funnels", { funnel: { id: 9 } });
    const deleted = mockApi("delete", "/v2/session/projects/42/funnels", { funnels: [] });
    await saveFunnel(session, "42", { name: "Checkout", steps, ...range });
    await deleteFunnel(session, "42", 9);
    expect(saved[0]?.body).toEqual({ name: "Checkout", steps, ...range });
    expect(queryOf(deleted[0])).toEqual({ funnel_id: "9" });
  });

  it("surfaces the server's error", async () => {
    mockApi("post", "/v2/session/projects/42/funnels", { error: "Name taken." }, 409);
    await expect(saveFunnel(session, "42", { name: "x", steps, ...range })).rejects.toThrow(
      "Name taken.",
    );
    mockApi("delete", "/v2/session/projects/42/funnels", {}, 500);
    await expect(deleteFunnel(session, "42", 1)).rejects.toThrow("Unable to delete funnel.");
  });
});

describe("database download client", () => {
  it("trades the session for a download token", async () => {
    const token = { token: "t-1", size_bytes: 2048, expires_at: 99 };
    const calls = mockApi("post", "/v2/session/projects/42/download-token", token);
    await expect(createDownloadToken(session, "42")).resolves.toEqual(token);
    expect(calls[0]?.credentials).toBe("include");
    mockApi("post", "/v2/session/projects/42/download-token", {}, 403);
    await expect(createDownloadToken(session, "42")).rejects.toThrow(
      "Unable to prepare the download.",
    );
  });

  it("puts the token in the download link", () => {
    expect(projectDatabaseDownloadUrl("42", "t-1")).toBe(
      "http://localhost:4320/v2/download/projects/42?token=t-1",
    );
  });

  it("names the file like the API's Content-Disposition", () => {
    expect(projectDatabaseFileName("42")).toBe("revscope-project-42.db");
  });

  it.each([
    [-5, "0 bytes"],
    [1023, "1023 bytes"],
    [1024, "1.0 kb"],
    [356_725_555, "340.2 mb"],
    [5 * 1024 ** 5, "5120.0 tb"],
  ])("formats %d bytes as %s", (bytes, expected) => {
    expect(formatFileSize(bytes)).toBe(expected);
  });
});
