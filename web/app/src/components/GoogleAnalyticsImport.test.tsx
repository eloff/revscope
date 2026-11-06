import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { testSession } from "../../../shared/test/fixtures";
import { stubLocation } from "../../../shared/test/location";
import {
  type GaConnection,
  type GaImport,
  type GaImportsResponse,
  storePendingConnection,
} from "../lib/gaImportApi";
import GoogleAnalyticsImport from "./GoogleAnalyticsImport";

const session = testSession();
const PATH = "/v2/session/projects/1/ga-imports";

function imports(overrides: Partial<GaImportsResponse> = {}): GaImportsResponse {
  return {
    oauth_available: true,
    oldest_allowed_date: "2026-05-04",
    latest_allowed_date: "2026-11-03",
    default_start_date: "2026-05-04",
    default_end_date: "2026-08-09",
    first_tracked_date: "2026-08-10",
    include_in_reports: true,
    imports: [],
    ...overrides,
  };
}

function job(overrides: Partial<GaImport> = {}): GaImport {
  return {
    id: 5,
    property_name: "Acme Web",
    start_date: "2026-05-04",
    end_date: "2026-08-09",
    hostnames: [],
    status: "completed",
    chunks_done: 28,
    chunks_total: 28,
    resume_at: "2026-11-03",
    error: null,
    sessions_imported: 1234,
    ...overrides,
  };
}

const connection: GaConnection = {
  connection_id: "conn-1",
  properties: [{ id: "123", name: "Acme Web", account: "Acme" }],
  known_hostnames: ["acme.test", "www.acme.test"],
};

const renderSection = () => render(() => <GoogleAnalyticsImport session={session} projectId="1" />);

beforeEach(() => {
  sessionStorage.clear();
  stubLocation("/settings");
});

/** Lists `current()` on GET, so an action's handler can change what's listed next. */
function mockList(current: () => GaImportsResponse) {
  return mockApi("get", PATH, () => HttpResponse.json(current()));
}

describe("GoogleAnalyticsImport", () => {
  it("offers both ways to connect, or only the key when OAuth isn't configured", async () => {
    mockList(() => imports());
    const first = renderSection();
    expect(await screen.findByRole("button", { name: "Connect with Google" })).toBeTruthy();
    expect(screen.getByLabelText("Service account JSON key")).toBeTruthy();
    first.unmount();

    mockList(() => imports({ oauth_available: false }));
    renderSection();
    expect(await screen.findByLabelText("Service account JSON key")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Connect with Google" })).toBeNull();
  });

  it("goes to Google's consent screen", async () => {
    const assign = stubLocation("/settings");
    mockList(() => imports());
    mockApi("post", `${PATH}/oauth-url`, { url: "https://accounts.google.test/consent" });
    renderSection();
    await userEvent.click(await screen.findByRole("button", { name: "Connect with Google" }));
    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith("https://accounts.google.test/consent"),
    );
  });

  it("sends a pasted key, then fills the form with the defaults", async () => {
    mockList(() => imports());
    const connects = mockApi("post", `${PATH}/connect`, connection);
    renderSection();
    await userEvent.click(await screen.findByLabelText("Service account JSON key"));
    await userEvent.paste('{"client_email":"a@b"}');
    await userEvent.click(screen.getByRole("button", { name: "Connect service account" }));

    expect(await screen.findByLabelText("From")).toHaveProperty("value", "2026-05-04");
    expect(connects[0].body).toEqual({ service_account_json: '{"client_email":"a@b"}' });
    expect(screen.getByLabelText("To")).toHaveProperty("value", "2026-08-09");
    expect(screen.getByLabelText("From")).toHaveProperty("min", "2026-05-04");
    expect(screen.getByLabelText("Hostnames")).toHaveProperty("value", "acme.test, www.acme.test");
  });

  it("explains a credential that can't read any property", async () => {
    storePendingConnection("1", { ...connection, properties: [] });
    mockList(() => imports());
    renderSection();
    expect(await screen.findByText(/can't read any GA4 properties/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Connect again" }));
    expect(await screen.findByLabelText("Service account JSON key")).toBeTruthy();
  });

  it("warns about duplicates only past the start of tracking, and acknowledges it on start", async () => {
    storePendingConnection("1", connection);
    mockList(() => imports());
    const starts = mockApi("post", PATH, job({ status: "running" }));
    renderSection();
    const to = await screen.findByLabelText("To");
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.input(to, { target: { value: "2026-08-10" } });
    expect((await screen.findByRole("alert")).textContent).toMatch(/duplicated/);

    await userEvent.click(screen.getByRole("button", { name: "Start import" }));
    await waitFor(() => expect(starts).toHaveLength(1));
    expect(starts[0].body).toEqual({
      connection_id: "conn-1",
      property_id: "123",
      start_date: "2026-05-04",
      end_date: "2026-08-10",
      hostnames: ["acme.test", "www.acme.test"],
      allow_duplicates: true,
    });
  });

  /** Past the first load, which keeps every button disabled whatever the dates say. */
  async function renderForm(overrides: Partial<GaImportsResponse> = {}) {
    storePendingConnection("1", connection);
    mockList(() => imports(overrides));
    renderSection();
    await screen.findByLabelText("To");
    await new Promise((resolve) => setTimeout(resolve));
  }

  const setDate = (label: "From" | "To", value: string) =>
    fireEvent.input(screen.getByLabelText(label), { target: { value } });
  const startButton = () => screen.getByRole("button", { name: "Start import" });
  const NOTHING = /nothing to import without duplicating/;
  const OUT_OF_ORDER = "The end date is before the start date.";
  const allTracked = { default_end_date: "2025-12-31", first_tracked_date: "2026-01-01" };

  it("offers nothing to start when tracking covers the whole retention period", async () => {
    const starts = mockApi("post", PATH, job({ status: "running" }));
    await renderForm(allTracked);
    expect(screen.getByText(NOTHING)).toBeTruthy();
    expect(screen.queryByText(OUT_OF_ORDER)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(startButton()).toHaveProperty("disabled", true);

    setDate("To", "2026-06-01");
    expect(screen.getByRole("alert").textContent).toMatch(/duplicated/);
    expect(screen.queryByText(NOTHING)).toBeNull();
    expect(startButton()).toHaveProperty("disabled", false);
    await userEvent.click(startButton());
    await waitFor(() => expect(starts).toHaveLength(1));
    expect(starts[0].body).toMatchObject({ end_date: "2026-06-01", allow_duplicates: true });
  });

  it("treats a chosen end date as a choice, even a wrong one", async () => {
    await renderForm(allTracked);
    setDate("To", "2026-01-15");
    expect(screen.queryByText(NOTHING)).toBeNull();
    expect(screen.getByText(OUT_OF_ORDER)).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toMatch(/duplicated/);
    expect(startButton()).toHaveProperty("disabled", true);
  });

  it("won't start a range that ends before it begins", async () => {
    await renderForm();
    expect(startButton()).toHaveProperty("disabled", false);
    setDate("From", "2026-09-01");
    expect(startButton()).toHaveProperty("disabled", true);
    expect(screen.getByText(OUT_OF_ORDER)).toBeTruthy();
    expect(screen.queryByText(NOTHING)).toBeNull();
  });

  it("won't start before the plan's retention, however the date got there", async () => {
    await renderForm();
    setDate("From", "2026-01-01");
    expect(startButton()).toHaveProperty("disabled", true);
    expect(screen.getByText("That start date is older than your plan keeps.")).toBeTruthy();
  });

  it("names retention, not the end date, when both are blocking", async () => {
    await renderForm(allTracked);
    // Older than the plan keeps, yet still after the default end.
    setDate("From", "2026-03-01");
    expect(screen.getByText("That start date is older than your plan keeps.")).toBeTruthy();
    expect(screen.queryByText(NOTHING)).toBeNull();
  });

  it("won't start a range reaching today, which isn't final", async () => {
    await renderForm();
    setDate("To", "2026-11-04");
    expect(startButton()).toHaveProperty("disabled", true);
    expect(screen.getByText(/must be before today/)).toBeTruthy();
  });

  it("asks for a date that's been cleared, on any plan", async () => {
    await renderForm();
    setDate("To", "");
    expect(screen.getByText("Choose an end date.")).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);
    setDate("To", "2026-08-09");
    setDate("From", "");
    expect(screen.getByText("Choose a start date.")).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);
  });

  it("asks for a cleared start on a plan that keeps everything", async () => {
    await renderForm({ oldest_allowed_date: null });
    setDate("From", "");
    expect(screen.getByText("Choose a start date.")).toBeTruthy();
    expect(startButton()).toHaveProperty("disabled", true);
  });

  it("doesn't acknowledge duplicates for a range before tracking", async () => {
    storePendingConnection("1", connection);
    mockList(() => imports());
    const starts = mockApi("post", PATH, job({ status: "running" }));
    renderSection();
    await userEvent.click(await screen.findByRole("button", { name: "Start import" }));
    await waitFor(() => expect(starts).toHaveLength(1));
    expect(starts[0].body).toMatchObject({ allow_duplicates: false });
  });

  it("shows a server's refusal", async () => {
    storePendingConnection("1", connection);
    mockList(() => imports());
    mockApi("post", PATH, { error: "this range overlaps an earlier import" }, 409);
    renderSection();
    await userEvent.click(await screen.findByRole("button", { name: "Start import" }));
    expect(await screen.findByText(/overlaps an earlier import/)).toBeTruthy();
  });

  it("lists progress, and cancels a running import", async () => {
    let current = job({ status: "running", chunks_done: 7 });
    mockList(() => imports({ imports: [current] }));
    const cancels = mockApi("post", `${PATH}/cancel`, () => {
      current = job({ status: "cancelled", chunks_done: 7 });
      return HttpResponse.json(current);
    });
    renderSection();
    expect(await screen.findByText(/Importing, 25% done/)).toBeTruthy();
    expect(screen.queryByLabelText("Service account JSON key")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText(/Cancelled at 25%/)).toBeTruthy();
    expect(cancels[0].url.searchParams.get("id")).toBe("5");
  });

  it("says a paused import will resume by itself", async () => {
    mockList(() => imports({ imports: [job({ status: "paused", chunks_done: 14 })] }));
    renderSection();
    expect(
      await screen.findByText(/out of Google Analytics quota, resumes automatically/),
    ).toBeTruthy();
  });

  it("shows why an import failed", async () => {
    mockList(() =>
      imports({ imports: [job({ status: "failed", error: "Google Analytics refused access." })] }),
    );
    renderSection();
    expect(await screen.findByText("Google Analytics refused access.")).toBeTruthy();
  });

  it("asks once more before deleting an import", async () => {
    let current = [job()];
    mockList(() => imports({ imports: current }));
    const deletes = mockApi("delete", PATH, () => {
      current = [];
      return HttpResponse.json({ id: 5 });
    });
    renderSection();
    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));
    expect(deletes).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Really delete?" }));
    await waitFor(() => expect(screen.queryByText("Acme Web")).toBeNull());
    expect(deletes[0].url.searchParams.get("id")).toBe("5");
  });

  it("saves the reports setting", async () => {
    mockList(() => imports({ imports: [job()] }));
    const saves = mockApi("post", `${PATH}/settings`, { include_in_reports: false });
    renderSection();
    await userEvent.click(await screen.findByLabelText("Include imported data in reports"));
    await waitFor(() => expect(saves).toHaveLength(1));
    expect(saves[0].body).toEqual({ include_in_reports: false });
  });
});
