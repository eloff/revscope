import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { sessionApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

const ACCOUNTS_PATH = sessionApiPath("site-admin/accounts");

/** The terms an account is on now. Times are unix seconds. */
export type SiteAdminSubscription = {
  plan: string;
  billing_period: string;
  regular_price_cents: number;
  actual_price_cents: number;
  discount_ends_at: number | null;
  /** Null for unmetered plans. */
  events_monthly_limit: number | null;
  started_at: number;
  trial_ends_at: number | null;
  /** `stripe` or `polar`; null when no provider bills it. */
  provider: string | null;
  customer_id: string | null;
  subscription_id: string | null;
  /** The provider's status, as it reported it. */
  status: string | null;
  current_period_end: number | null;
};

export type SiteAdminUser = {
  user_id: string;
  name: string;
  email: string;
  is_admin: boolean;
  /** Invited but hasn't chosen a password yet. */
  pending: boolean;
  /** When the user was made: their signup, or for an invitee their first invite. */
  signed_up_at: number | null;
  /** Joined by invite: anyone but the account's earliest remaining user. */
  invited: boolean;
  /**
   * When they followed the link that confirms their address, or else when they
   * last proved it another way: a password reset link, or a Google or Apple
   * sign-in at an address the provider hosts.
   */
  email_verified_at: number | null;
  /** Google or Apple, linked to the user. */
  sign_in_providers: string[];
  last_login_at: number | null;
};

export type SiteAdminAccount = {
  account_id: string;
  name: string;
  signed_up_at: number | null;
  /** The signed-in admin's own account, which can't be deleted. */
  is_own: boolean;
  subscription: SiteAdminSubscription | null;
  users: SiteAdminUser[];
  /** By name. */
  projects: SiteAdminProject[];
};

export type SiteAdminProject = {
  id: number;
  name: string;
  /**
   * Tracked visitors and revenue (thousandths of the project's currency) in the
   * 30 days up to the request; null when the server couldn't read its database.
   */
  last_30_days: { visitors: number; revenue: number } | null;
};

export type SiteAdminAccountsPage = {
  /** Newest first. */
  accounts: SiteAdminAccount[];
  total: number;
  offset: number;
  page_size: number;
};

export async function fetchSiteAdminAccounts(
  session: AuthSession,
  offset: number,
): Promise<SiteAdminAccountsPage> {
  const url = new URL(ACCOUNTS_PATH, getApiBaseUrl());
  url.searchParams.set("offset", String(offset));
  const page = await fetchJson<Partial<SiteAdminAccountsPage>>(
    url,
    authRequestInit(session),
    "Unable to load accounts.",
  );
  if (!Array.isArray(page.accounts)) {
    throw new Error("Unable to load accounts.");
  }
  return {
    accounts: page.accounts,
    total: page.total ?? page.accounts.length,
    offset: page.offset ?? offset,
    page_size: page.page_size ?? page.accounts.length,
  };
}

/**
 * Permanently deletes an account with its users, projects and their data. The
 * server refuses unless `confirmName` is the account's name.
 */
export async function deleteSiteAdminAccount(
  session: AuthSession,
  accountId: string,
  confirmName: string,
): Promise<void> {
  await fetchJson<unknown>(
    new URL(ACCOUNTS_PATH, getApiBaseUrl()),
    authRequestInit(session, {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ account_id: accountId, confirm_name: confirmName }),
    }),
    "Unable to delete the account.",
  );
}
