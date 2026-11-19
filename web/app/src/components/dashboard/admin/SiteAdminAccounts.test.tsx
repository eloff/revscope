import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../../../shared/test/api";
import { testSession } from "../../../../../shared/test/fixtures";
import type { SiteAdminAccount, SiteAdminUser } from "../../../lib/siteAdminApi";
import SiteAdminAccounts from "./SiteAdminAccounts";

const ACCOUNTS = "/v2/session/site-admin/accounts";
// Noon UTC, so the date reads the same in every test runner's zone.
const MAY_1 = Date.UTC(2026, 4, 1, 12) / 1000;
const MAY_3 = Date.UTC(2026, 4, 3, 12) / 1000;

function user(overrides: Partial<SiteAdminUser> = {}): SiteAdminUser {
  return {
    user_id: "u1",
    name: "Grace Hopper",
    email: "grace@acme.test",
    is_admin: true,
    pending: false,
    signed_up_at: MAY_1,
    invited: false,
    email_verified_at: null,
    sign_in_providers: [],
    last_login_at: null,
    ...overrides,
  };
}

function account(overrides: Partial<SiteAdminAccount> = {}): SiteAdminAccount {
  return {
    account_id: "0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f",
    name: "acme.test",
    signed_up_at: MAY_1,
    is_own: false,
    subscription: {
      plan: "growth",
      billing_period: "monthly",
      regular_price_cents: 2900,
      actual_price_cents: 1900,
      discount_ends_at: null,
      events_monthly_limit: 1_000_000,
      started_at: MAY_1,
      trial_ends_at: null,
      provider: "stripe",
      customer_id: "cus_123",
      subscription_id: "sub_456",
      status: "active",
      current_period_end: MAY_3,
    },
    users: [user()],
    projects: [{ id: 7, name: "acmetest", last_30_days: { visitors: 1204, revenue: 3_456_780 } }],
    ...overrides,
  };
}

function page(accounts: SiteAdminAccount[], total = accounts.length, offset = 0) {
  return { accounts, total, offset, page_size: 100 };
}

function renderAccounts() {
  render(() => <SiteAdminAccounts session={testSession({ is_site_admin: true })} />);
}

describe("SiteAdminAccounts", () => {
  it("shows each account's users, projects, plan and subscription", async () => {
    const verified = user({
      user_id: "u2",
      name: "Ada Lovelace",
      email: "ada@acme.test",
      is_admin: false,
      invited: true,
      email_verified_at: MAY_3,
    });
    const google = user({
      user_id: "u3",
      name: "Alan Turing",
      email: "alan@acme.test",
      is_admin: false,
      sign_in_providers: ["google"],
    });
    const unreadable = { id: 8, name: "acmeblog", last_30_days: null };
    mockApi(
      "get",
      ACCOUNTS,
      page([
        account({
          users: [user(), verified, google],
          projects: [account().projects[0], unreadable],
        }),
      ]),
    );
    renderAccounts();

    const row = await screen.findByRole("row", { name: "acme.test" });
    const cells = within(row);
    expect(cells.getByText("0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f")).toBeInTheDocument();
    expect(cells.getAllByText(/^Signed up May 1, 2026$/)).toHaveLength(3);
    expect(cells.getByText("Invited May 1, 2026")).toBeInTheDocument();
    expect(cells.getByText("grace@acme.test")).toBeInTheDocument();
    expect(cells.getByText("Email not verified")).toBeInTheDocument();
    expect(cells.getByText("Verified May 3, 2026")).toBeInTheDocument();
    expect(cells.getByText("Verified by Google sign-in")).toBeInTheDocument();
    expect(cells.getByText("acmetest")).toBeInTheDocument();
    expect(cells.getByText("1,204 visitors · $3.5k revenue in 30 days")).toBeInTheDocument();
    expect(cells.getByText("Last 30 days unavailable")).toBeInTheDocument();
    expect(cells.getByText("Growth")).toBeInTheDocument();
    expect(cells.getByText("$19.00 / month (list $29.00)")).toBeInTheDocument();
    expect(cells.getByText("1,000,000 events / month")).toBeInTheDocument();
    expect(cells.getByText("Stripe · active")).toBeInTheDocument();
    expect(cells.getByText("Paid through May 3, 2026")).toBeInTheDocument();
    expect(cells.getByText("sub_456")).toBeInTheDocument();
  });

  it("pages through the accounts a hundred at a time", async () => {
    const calls = mockApi("get", ACCOUNTS, ({ url }) => {
      const offset = Number(url.searchParams.get("offset"));
      const name = offset === 0 ? "first.test" : "second.test";
      return HttpResponse.json(page([account({ name })], 150, offset));
    });
    renderAccounts();
    expect(await screen.findByText("1–1 of 150 accounts")).toBeInTheDocument();
    expect(queryOf(calls[0])).toEqual({ offset: "0" });

    await userEvent.setup().click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByRole("row", { name: "second.test" })).toBeInTheDocument();
    expect(queryOf(calls[1])).toEqual({ offset: "100" });
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  });

  it("deletes an account once its name is typed, then reloads the page", async () => {
    let remaining = [account(), account({ account_id: "other", name: "other.test" })];
    const listed = mockApi("get", ACCOUNTS, () => HttpResponse.json(page(remaining)));
    const deleted = mockApi("delete", ACCOUNTS, () => {
      remaining = remaining.filter((left) => left.name !== "acme.test");
      return HttpResponse.json({ deleted: "x", users_deleted: 1, projects_deleted: 1 });
    });
    renderAccounts();
    const user = userEvent.setup();

    const row = await screen.findByRole("row", { name: "acme.test" });
    await user.click(within(row).getByRole("button", { name: "Actions for acme.test" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete account…" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete acme.test?" });
    const confirm = within(dialog).getByRole("button", { name: "Delete account" });
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByLabelText("Type acme.test to confirm"), "acme.test");
    await user.click(confirm);

    await vi.waitFor(() => expect(screen.queryByRole("row", { name: "acme.test" })).toBeNull());
    expect(deleted[0]?.body).toEqual({
      account_id: "0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f",
      confirm_name: "acme.test",
    });
    expect(listed).toHaveLength(2);
    expect(screen.getByRole("row", { name: "other.test" })).toBeInTheDocument();
  });

  it("keeps the dialog open with the server's reason when it refuses", async () => {
    mockApi("get", ACCOUNTS, page([account()]));
    mockApi(
      "delete",
      ACCOUNTS,
      { error: "This account has a Stripe or Polar subscription that hasn't been canceled." },
      409,
    );
    renderAccounts();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Actions for acme.test" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete account…" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "acme.test");
    await user.click(within(dialog).getByRole("button", { name: "Delete account" }));
    expect(
      await within(dialog).findByText(/subscription that hasn't been canceled/),
    ).toBeInTheDocument();
  });

  it("keeps the last page up when the next fails, and tries again", async () => {
    let failing = true;
    mockApi("get", ACCOUNTS, ({ url }) => {
      const offset = Number(url.searchParams.get("offset"));
      if (offset > 0 && failing) {
        return HttpResponse.json({ error: "Unable to load accounts." }, { status: 500 });
      }
      const name = offset === 0 ? "first.test" : "second.test";
      return HttpResponse.json(page([account({ name })], 150, offset));
    });
    renderAccounts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(await screen.findByText("Unable to load accounts.")).toBeInTheDocument();
    expect(screen.getByRole("row", { name: "first.test" })).toBeInTheDocument();

    failing = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("row", { name: "second.test" })).toBeInTheDocument();
    expect(screen.queryByText("Unable to load accounts.")).toBeNull();
  });

  it("reloads the page on screen after a delete, even when the next one failed", async () => {
    const failing = true;
    let firstPage = [account({ name: "first.test" }), account({ account_id: "b", name: "b.test" })];
    const calls = mockApi("get", ACCOUNTS, ({ url }) => {
      const offset = Number(url.searchParams.get("offset"));
      if (offset > 0 && failing) {
        return HttpResponse.json({ error: "Unable to load accounts." }, { status: 500 });
      }
      const accounts = offset === 0 ? firstPage : [account({ name: "second.test" })];
      return HttpResponse.json(page(accounts, 150, offset));
    });
    mockApi("delete", ACCOUNTS, () => {
      firstPage = firstPage.filter((left) => left.name !== "b.test");
      return HttpResponse.json({ deleted: "b", users_deleted: 1, projects_deleted: 1 });
    });
    renderAccounts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(await screen.findByText("Unable to load accounts.")).toBeInTheDocument();

    // Deleting from the page still on screen reloads that page, not the one that failed.
    await user.click(screen.getByRole("button", { name: "Actions for b.test" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete account…" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByRole("textbox"), "b.test");
    await user.click(within(dialog).getByRole("button", { name: "Delete account" }));
    await vi.waitFor(() => expect(screen.queryByRole("row", { name: "b.test" })).toBeNull());
    expect(queryOf(calls.at(-1))).toEqual({ offset: "0" });
  });

  it("retries a failed Next when Next is pressed again", async () => {
    let failing = true;
    mockApi("get", ACCOUNTS, ({ url }) => {
      const offset = Number(url.searchParams.get("offset"));
      if (offset > 0 && failing) {
        return HttpResponse.json({ error: "Unable to load accounts." }, { status: 500 });
      }
      const name = offset === 0 ? "first.test" : "second.test";
      return HttpResponse.json(page([account({ name })], 150, offset));
    });
    renderAccounts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(await screen.findByText("Unable to load accounts.")).toBeInTheDocument();

    failing = false;
    await user.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByRole("row", { name: "second.test" })).toBeInTheDocument();
  });

  it("offers no actions on the admin's own account", async () => {
    mockApi("get", ACCOUNTS, page([account({ is_own: true })]));
    renderAccounts();
    const row = await screen.findByRole("row", { name: /acme\.test/ });
    expect(within(row).getByText("You")).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "Actions for acme.test" })).toBeDisabled();
  });
});
