import { setSelectedProjectId } from "@revscope/web-shared/projectSelection";
import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { signIn, signOut } from "../../../shared/test/auth";
import Troubleshooting from "./Troubleshooting";

const BASE = "/v2/session/projects/42/troubleshooting";
const status = {
  project_name: "example_com",
  default_url: "https://example.com/",
  events_last_month: 12_345,
  events_queued: 0,
  integrations: [
    {
      integration_type: "stripe",
      last_synced_at: "2026-01-01T00:00:00Z",
      last_sync_error: null,
      status: "ok",
    },
    {
      integration_type: "cloudflare",
      last_synced_at: null,
      last_sync_error: "Bad token",
      status: "error",
    },
  ],
};
const check = {
  url: "https://example.com/",
  fetch: { ok: true, status_code: 200, error: null, note: null },
  install: { ok: true, findings: [{ severity: "warning", message: "Snippet is below the fold." }] },
  wait_started_at_ms: 1000,
};

beforeEach(() => setSelectedProjectId("42"));

afterEach(async () => {
  vi.useRealTimers();
  setSelectedProjectId("");
  await signOut();
});

async function renderLoaded(loaded = status) {
  signIn();
  mockApi("get", BASE, loaded);
  render(() => <Troubleshooting />);
  await screen.findByText(new Intl.NumberFormat().format(12_345));
  return userEvent.setup();
}

describe("Troubleshooting", () => {
  it("asks a signed-out visitor to log in", async () => {
    render(() => <Troubleshooting />);
    expect(await screen.findByText("Log in to run troubleshooting.")).toBeInTheDocument();
  });

  it("shows recent ingest, the default URL and integration health", async () => {
    await renderLoaded();
    expect(screen.getByText("example_com")).toBeInTheDocument();
    expect(screen.getByLabelText("URL to check")).toHaveValue("https://example.com/");
    expect(screen.getByText("Stripe")).toBeInTheDocument();
    expect(screen.getByText("Synced")).toBeInTheDocument();
    expect(screen.getByText("Bad token")).toBeInTheDocument();
  });

  it("says when received events are waiting to be saved, and not when none are", async () => {
    await renderLoaded();
    expect(screen.queryByText(/waiting to be saved/)).toBeNull();
    cleanup();
    await renderLoaded({ ...status, events_queued: 3 });
    expect(screen.getByText(/waiting to be saved/)).toBeInTheDocument();
  });

  it("falls back to settings for integrations when status fails", async () => {
    signIn();
    mockApi("get", BASE, { error: "Status unavailable." }, 500);
    mockApi("get", "/v2/session/projects/42/settings", {
      timezone: "UTC",
      payment_success_path: null,
      custom_domain: null,
      integrations: [
        {
          integration_type: "polar",
          external_account_id: null,
          api_key_masked: "x",
          secret_masked: null,
          last_synced_at: null,
          last_sync_error: null,
        },
      ],
    });
    render(() => <Troubleshooting />);
    expect(await screen.findByText("Status unavailable.")).toBeInTheDocument();
    expect(await screen.findByText("Polar")).toBeInTheDocument();
    expect(screen.getByText("Waiting for first sync")).toBeInTheDocument();
    expect(screen.getByText("Event count unavailable.")).toBeInTheDocument();
  });

  it("keeps showing the last failure while a sync runs", async () => {
    signIn();
    mockApi("get", BASE, { error: "Status unavailable." }, 500);
    mockApi("get", "/v2/session/projects/42/settings", {
      timezone: "UTC",
      integrations: [
        {
          integration_type: "polar",
          external_account_id: null,
          api_key_masked: "x",
          secret_masked: null,
          last_synced_at: "2026-11-01T00:00:00Z",
          last_sync_error: null,
          last_finished_sync_error: "Key revoked",
        },
      ],
    });
    render(() => <Troubleshooting />);
    expect(await screen.findByText("Key revoked")).toBeInTheDocument();
  });

  it("checks the page, then reports the first event from it", async () => {
    const user = await renderLoaded();
    const checks = mockApi("post", `${BASE}/check`, check);
    const polls = mockApi("get", `${BASE}/ingest`, {
      detected: true,
      detected_at_ms: 5000,
      path: "/pricing",
      hostname: "example.com",
    });
    await user.click(screen.getByRole("button", { name: "Check installation" }));

    expect(await screen.findByText("Fetched https://example.com/ (HTTP 200).")).toBeInTheDocument();
    expect(screen.getByText("Snippet is below the fold.")).toBeInTheDocument();
    expect(
      await screen.findByText(/Event successfully detected at .*\(example\.com\/pricing\)\./),
    ).toBeInTheDocument();
    expect(checks[0]?.body).toEqual({ url: "https://example.com/" });
    expect(queryOf(polls[0])).toEqual({ since_ms: "1000" });
  });

  it("reports a page that couldn't be fetched", async () => {
    const user = await renderLoaded();
    mockApi("post", `${BASE}/check`, {
      ...check,
      fetch: { ok: false, status_code: null, error: "DNS failure", note: "Is the site public?" },
    });
    mockApi("get", `${BASE}/ingest`, {
      detected: true,
      detected_at_ms: 1,
      path: null,
      hostname: null,
    });
    await user.click(screen.getByRole("button", { name: "Check installation" }));
    expect(
      await screen.findByText(/Could not fetch https:\/\/example.com\/: DNS failure/),
    ).toHaveTextContent("Is the site public?");
  });

  it("gives up waiting for an event after two minutes", async () => {
    const user = await renderLoaded();
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"], shouldAdvanceTime: true });
    mockApi("post", `${BASE}/check`, check);
    const polls = mockApi("get", `${BASE}/ingest`, {
      detected: false,
      detected_at_ms: null,
      path: null,
      hostname: null,
    });
    await user.click(screen.getByRole("button", { name: "Check installation" }));
    expect(await screen.findByText(/Waiting for an event from your site/)).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(2 * 60 * 1000 + 2000);
    expect(await screen.findByText(/No ingest event arrived within 2 minutes/)).toBeInTheDocument();
    const count = polls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(polls.length).toBe(count);
  });

  it("shows a failed check", async () => {
    const user = await renderLoaded();
    mockApi("post", `${BASE}/check`, { error: "URL must be on your site." }, 400);
    const field = screen.getByLabelText("URL to check");
    await user.clear(field);
    expect(screen.getByRole("button", { name: "Check installation" })).toBeDisabled();
    await user.type(field, "https://other.example/");
    await user.click(screen.getByRole("button", { name: "Check installation" }));
    expect(await screen.findByText("URL must be on your site.")).toBeInTheDocument();
  });
});
