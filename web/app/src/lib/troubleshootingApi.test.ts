import { HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { testSession } from "../../../shared/test/fixtures";
import {
  fetchTroubleshootingStatus,
  formatEventsCount,
  formatSyncAt,
  integrationStatusLabel,
  integrationTypeLabel,
  pollTroubleshootingIngest,
  runTroubleshootingCheck,
} from "./troubleshootingApi";

const session = testSession();
const BASE = "/v2/session/projects/42/troubleshooting";

describe("troubleshooting client", () => {
  it("loads status, runs a check and polls ingest", async () => {
    const status = mockApi("get", BASE, { integrations: [] });
    const check = mockApi("post", `${BASE}/check`, { ok: true });
    const ingest = mockApi("get", `${BASE}/ingest`, { detected: false });

    await expect(fetchTroubleshootingStatus(session, "42")).resolves.toEqual({ integrations: [] });
    await runTroubleshootingCheck(session, "42", "https://example.com");
    await pollTroubleshootingIngest(session, "42", 1234);

    expect(status[0]?.credentials).toBe("include");
    expect(check[0]?.body).toEqual({ url: "https://example.com" });
    expect(queryOf(ingest[0])).toEqual({ since_ms: "1234" });
  });

  it("surfaces the server's error or the fallback", async () => {
    mockApi("post", `${BASE}/check`, { error: "URL is not on your site." }, 400);
    await expect(runTroubleshootingCheck(session, "42", "x")).rejects.toThrow(
      "URL is not on your site.",
    );
    mockApi("get", `${BASE}/ingest`, {}, 500);
    await expect(pollTroubleshootingIngest(session, "42", 0)).rejects.toThrow(
      "Unable to poll ingest.",
    );
  });

  it("uses the fallback when the body isn't JSON", async () => {
    mockApi("get", BASE, () => new HttpResponse("<html>", { status: 200 }));
    await expect(fetchTroubleshootingStatus(session, "42")).rejects.toThrow(
      "Unable to load troubleshooting status.",
    );
  });

  it("names the API origin when the network fails", async () => {
    mockApi("get", BASE, () => HttpResponse.error());
    await expect(fetchTroubleshootingStatus(session, "42")).rejects.toThrow(
      "Unable to reach the API at http://localhost:4320. Is the backend running?",
    );
  });
});

describe("troubleshooting labels", () => {
  it("labels integration types, humanizing unknown ones", () => {
    expect(integrationTypeLabel("lemon_squeezy")).toBe("Lemon Squeezy");
    expect(integrationTypeLabel("paddle_billing")).toBe("paddle billing");
  });

  it("labels sync statuses", () => {
    expect(integrationStatusLabel("ok")).toBe("Synced");
    expect(integrationStatusLabel("error")).toBe("Error");
    expect(integrationStatusLabel("never_run")).toBe("Waiting for first sync");
    expect(integrationStatusLabel("paused")).toBe("paused");
  });

  it("formats sync times, passing through what it can't parse", () => {
    expect(formatSyncAt(null)).toBe("—");
    expect(formatSyncAt("not a date")).toBe("not a date");
    expect(formatSyncAt("2026-01-02T03:04:05Z")).toMatch(/2026/);
  });

  it("groups event counts", () => {
    expect(formatEventsCount(1234567)).toBe(new Intl.NumberFormat().format(1234567));
  });
});
