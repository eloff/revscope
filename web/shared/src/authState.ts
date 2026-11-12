import { createSignal, onMount } from "solid-js";
import { getApiBaseUrl } from "./apiBaseUrl";
import { publicApiPath, sessionApiPath } from "./apiPaths";
import { SELF_HOSTED } from "./links";

/** Matches the tracker stub's default dispatch name. No-ops when the snippet is absent. */
const TRACKER_DISPATCH_NAME = "dispatch";

function identifyTrackerUser(user: { user_id: string; email: string; account_id: string }): void {
  if (typeof window === "undefined") {
    return;
  }
  const dispatch = (window as unknown as Record<string, unknown>)[TRACKER_DISPATCH_NAME];
  if (typeof dispatch !== "function") {
    return;
  }
  (dispatch as (command: "metadata", metadata: Record<string, string>) => void)("metadata", {
    user_id: user.user_id,
    user_email: user.email,
    account_id: user.account_id,
  });
}

export type ProjectSettings = {
  timezone: string;
};

export type AuthProject = {
  id: string;
  settings: ProjectSettings;
};

/**
 * The `/v2/session/me` payload: who the session cookie belongs to. The session id
 * itself never reaches JavaScript; it lives in an `HttpOnly` cookie that the
 * browser attaches to API requests made with `credentials: "include"`.
 */
export type AuthSession = {
  name: string;
  email: string;
  account_id: string;
  user_id: string;
  /** Project info keyed by project name. */
  projects: Record<string, AuthProject>;
  plan: string;
  /** Admins see billing and manage the account's users. */
  is_admin: boolean;
  /**
   * Whether someone on the account confirmed their email address. Ask AI, GA
   * imports and integrations wait for it.
   */
  account_confirmed: boolean;
  gravatar_url?: string;
  /**
   * Months of history the dashboards show, counted back from today; `null`
   * when they show all of it.
   */
  data_retention_months: number | null;
  /**
   * A paid subscription that ended unpaid and moved the account onto the free
   * plan. Sent to admins only, since only they can pay it.
   */
  unpaid_subscription: UnpaidSubscription | null;
};

/** What restoring a subscription that ended unpaid costs: one period of it, from the day it's paid. */
export type UnpaidSubscription = {
  plan: string;
  /** `null` means the plan is unmetered. */
  events_monthly_limit: number | null;
  billing_period: string;
  amount_cents: number;
  currency: string;
  /** When the account moved onto the free plan, in unix seconds. */
  ended_at: number;
};

type StoredAuthSession = {
  user: AuthSession;
  /** Epoch milliseconds of the `/v2/session/me` response this was built from. */
  fetched_at: number;
};

const AUTH_SESSION_STORAGE_KEY = "revscope.auth.session";
/** How long a stored `/v2/session/me` payload is served before it is re-fetched. */
const AUTH_REFRESH_INTERVAL_MS = 5 * 60 * 1000;

function isProjectSettings(value: unknown): value is ProjectSettings {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    typeof (value as ProjectSettings).timezone === "string" &&
    Boolean((value as ProjectSettings).timezone)
  );
}

function isAuthProject(value: unknown): value is AuthProject {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    typeof (value as AuthProject).id === "string" &&
    Boolean((value as AuthProject).id) &&
    isProjectSettings((value as AuthProject).settings)
  );
}

function isProjectsRecord(value: unknown): value is Record<string, AuthProject> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.values(value).every(isAuthProject)
  );
}

function parseUnpaidSubscription(value: unknown): UnpaidSubscription | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const unpaid = value as Record<string, unknown>;
  if (
    typeof unpaid.plan !== "string" ||
    !(unpaid.events_monthly_limit === null || typeof unpaid.events_monthly_limit === "number") ||
    typeof unpaid.billing_period !== "string" ||
    typeof unpaid.amount_cents !== "number" ||
    typeof unpaid.currency !== "string" ||
    typeof unpaid.ended_at !== "number"
  ) {
    return null;
  }
  return {
    plan: unpaid.plan,
    events_monthly_limit: unpaid.events_monthly_limit,
    billing_period: unpaid.billing_period,
    amount_cents: unpaid.amount_cents,
    currency: unpaid.currency,
    ended_at: unpaid.ended_at,
  };
}

function unpaidSubscriptionsEqual(
  left: UnpaidSubscription | null,
  right: UnpaidSubscription | null,
): boolean {
  if (!left || !right) {
    return left === right;
  }
  return (
    left.plan === right.plan &&
    left.events_monthly_limit === right.events_monthly_limit &&
    left.billing_period === right.billing_period &&
    left.amount_cents === right.amount_cents &&
    left.currency === right.currency &&
    left.ended_at === right.ended_at
  );
}

/** Validates a `/v2/session/me` payload, from the network or from localStorage. */
export function parseAuthSession(value: unknown): AuthSession | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const payload = value as Record<string, unknown>;
  if (
    typeof payload.name !== "string" ||
    !payload.name ||
    typeof payload.email !== "string" ||
    !payload.email ||
    typeof payload.account_id !== "string" ||
    typeof payload.user_id !== "string" ||
    typeof payload.plan !== "string" ||
    typeof payload.is_admin !== "boolean" ||
    !isProjectsRecord(payload.projects)
  ) {
    return null;
  }
  return {
    name: payload.name,
    email: payload.email,
    account_id: payload.account_id,
    user_id: payload.user_id,
    projects: payload.projects,
    plan: payload.plan,
    is_admin: payload.is_admin,
    // Absent from servers that don't verify, and from sessions stored before it was sent.
    account_confirmed: payload.account_confirmed !== false,
    gravatar_url: typeof payload.gravatar_url === "string" ? payload.gravatar_url : undefined,
    // Absent means unlimited, as from a server that keeps everything.
    data_retention_months:
      typeof payload.data_retention_months === "number" ? payload.data_retention_months : null,
    unpaid_subscription: parseUnpaidSubscription(payload.unpaid_subscription),
  };
}

function readStoredAuthSession(): StoredAuthSession | null {
  if (typeof window === "undefined") {
    return null;
  }
  const rawValue = window.localStorage.getItem(AUTH_SESSION_STORAGE_KEY);
  if (!rawValue) {
    return null;
  }
  // Anything unreadable is dropped, which also clears out sessions stored
  // before the session id moved into a cookie.
  try {
    const stored = JSON.parse(rawValue) as Partial<StoredAuthSession>;
    const user = parseAuthSession(stored?.user);
    if (!user) {
      window.localStorage.removeItem(AUTH_SESSION_STORAGE_KEY);
      return null;
    }
    return {
      user,
      fetched_at: typeof stored.fetched_at === "number" ? stored.fetched_at : 0,
    };
  } catch {
    window.localStorage.removeItem(AUTH_SESSION_STORAGE_KEY);
    return null;
  }
}

function writeStoredAuthSession(stored: StoredAuthSession | null): void {
  if (typeof window === "undefined") {
    return;
  }
  if (stored) {
    window.localStorage.setItem(AUTH_SESSION_STORAGE_KEY, JSON.stringify(stored));
    return;
  }
  window.localStorage.removeItem(AUTH_SESSION_STORAGE_KEY);
}

function projectsEqual(
  left: Record<string, AuthProject>,
  right: Record<string, AuthProject>,
): boolean {
  if (left === right) {
    return true;
  }
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }
  return leftKeys.every((key) => {
    const leftProject = left[key];
    const rightProject = right[key];
    return (
      leftProject !== undefined &&
      rightProject !== undefined &&
      leftProject.id === rightProject.id &&
      leftProject.settings.timezone === rightProject.settings.timezone
    );
  });
}

/** Value equality so refreshes that return the same account don't re-render. */
function authSessionsEqual(left: AuthSession | null, right: AuthSession): boolean {
  if (!left) {
    return false;
  }
  return (
    left.name === right.name &&
    left.email === right.email &&
    left.account_id === right.account_id &&
    left.user_id === right.user_id &&
    left.plan === right.plan &&
    left.is_admin === right.is_admin &&
    left.account_confirmed === right.account_confirmed &&
    left.gravatar_url === right.gravatar_url &&
    left.data_retention_months === right.data_retention_months &&
    unpaidSubscriptionsEqual(left.unpaid_subscription, right.unpaid_subscription) &&
    projectsEqual(left.projects, right.projects)
  );
}

// Start unresolved so SSR and the client's first paint match (no localStorage on the
// server). Each island hydrates from localStorage in onMount.
const [authSession, setAuthSession] = createSignal<AuthSession | null>(null);
const [authResolved, setAuthResolved] = createSignal(false);
let lastFetchedAt = 0;
let refreshInFlight: Promise<AuthSession | null> | null = null;
let refreshWatcherStarted = false;

export function saveAuthSession(nextSession: AuthSession): void {
  lastFetchedAt = Date.now();
  writeStoredAuthSession({ user: nextSession, fetched_at: lastFetchedAt });
  if (!authSessionsEqual(authSession(), nextSession)) {
    setAuthSession(nextSession);
  }
  setAuthResolved(true);
  identifyTrackerUser(nextSession);
}

/** Keeps the cached `/me` projects map in sync after saving project settings. */
export function patchProjectTimezone(projectId: string, timezone: string): void {
  const current = authSession();
  if (!current) {
    return;
  }
  let changed = false;
  const projects: Record<string, AuthProject> = {};
  for (const [name, project] of Object.entries(current.projects)) {
    if (project.id === projectId && project.settings.timezone !== timezone) {
      projects[name] = {
        ...project,
        settings: { ...project.settings, timezone },
      };
      changed = true;
    } else {
      projects[name] = project;
    }
  }
  if (changed) {
    saveAuthSession({ ...current, projects });
  }
}

function clearAuthSession(): void {
  lastFetchedAt = 0;
  setAuthSession(null);
  writeStoredAuthSession(null);
  setAuthResolved(true);
}

/** Ends the session server-side, then drops the cached account. */
export async function logOut(): Promise<void> {
  try {
    await fetch(new URL(publicApiPath("auth/logout"), getApiBaseUrl()), {
      method: "POST",
      credentials: "include",
    });
  } finally {
    clearAuthSession();
  }
}

/**
 * Reloads `/v2/session/me`. The stored copy is only replaced when the account
 * actually changed, so subscribers stay put on an unchanged refresh.
 */
function refreshAuthSession(): Promise<AuthSession | null> {
  refreshInFlight ??= (async () => {
    try {
      const response = await fetch(new URL(sessionApiPath("me"), getApiBaseUrl()), {
        credentials: "include",
      });
      // A rejected session is worthless, so drop it rather than leaving the
      // dashboard retrying it on every page.
      if (response.status === 401) {
        clearAuthSession();
        return null;
      }
      const session = response.ok ? parseAuthSession(await response.json()) : null;
      if (!session) {
        return authSession();
      }
      saveAuthSession(session);
      return authSession();
    } catch {
      // Offline or a server hiccup: the cached account is still the best answer.
      return authSession();
    } finally {
      refreshInFlight = null;
    }
  })();
  return refreshInFlight;
}

/**
 * Re-checks the cached account after the API refused it, so a revoked or
 * expired session signs the tab out now rather than at the next refresh. A
 * tab that is already signed out has nothing to re-check.
 */
export function revalidateAuthSession(): Promise<AuthSession | null> {
  if (!authSession()) {
    return Promise.resolve(null);
  }
  return refreshAuthSession();
}

/**
 * Reloads the account after this tab changed it, such as adding or deleting a
 * project. A refresh already in flight may have been answered before the
 * change, so this waits it out and fetches again rather than joining it.
 */
export async function reloadAuthSession(): Promise<AuthSession | null> {
  if (refreshInFlight) {
    await refreshInFlight;
  }
  return refreshAuthSession();
}

/**
 * Refreshes a cached account that may have gone stale. Signed-out visitors are
 * left alone so the marketing pages don't call the API at all; self-hosted
 * pages check the cookie once on load instead (see useAuthState).
 */
export function refreshAuthSessionIfStale(): void {
  if (!authSession() || Date.now() - lastFetchedAt < AUTH_REFRESH_INTERVAL_MS) {
    return;
  }
  void refreshAuthSession();
}

/** Catches up a tab that sat in the background past the refresh interval. */
function startAuthRefreshWatcher(): void {
  if (refreshWatcherStarted || typeof document === "undefined") {
    return;
  }
  refreshWatcherStarted = true;
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      refreshAuthSessionIfStale();
    }
  });
}

export function useAuthState() {
  onMount(() => {
    if (!authResolved()) {
      const stored = readStoredAuthSession();
      if (stored) {
        lastFetchedAt = stored.fetched_at;
        setAuthSession(stored.user);
        identifyTrackerUser(stored.user);
        setAuthResolved(true);
      } else if (SELF_HOSTED) {
        // Every self-hosted page is the app, so ask the cookie rather than
        // trusting a missing copy: a cleared or never-written localStorage
        // would otherwise sign out a valid session.
        void refreshAuthSession().finally(() => setAuthResolved(true));
      } else {
        setAuthResolved(true);
      }
    }
    refreshAuthSessionIfStale();
    startAuthRefreshWatcher();
  });

  return {
    authUser: authSession,
    authResolved,
  };
}
