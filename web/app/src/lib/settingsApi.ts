import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath, sessionApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

/** Integration types, mirrors the `integration_type` enum in revscope. */
export const INTEGRATION_TYPES = [
  "stripe",
  "shopify",
  "square",
  "lemon_squeezy",
  "polar",
  "dodo",
  "google_search_console",
  "cloudflare",
] as const;

export type IntegrationType = (typeof INTEGRATION_TYPES)[number];

/** Which `integrations` column a form field maps to. */
export type IntegrationColumn = "api_key" | "secret" | "external_account_id";

export type IntegrationField = {
  column: IntegrationColumn;
  label: string;
  placeholder?: string;
  help?: string;
  /** Secret fields are stored encrypted and only ever shown masked. */
  secret: boolean;
};

export type IntegrationTypeMeta = {
  type: IntegrationType;
  label: string;
  description: string;
  fields: IntegrationField[];
};

/**
 * Form definition per integration type. The credential values map onto the
 * generic `integrations` columns (`encrypted_api_key`, `encrypted_secret`,
 * `external_account_id`).
 */
export const INTEGRATION_META: Record<IntegrationType, IntegrationTypeMeta> = {
  stripe: {
    type: "stripe",
    label: "Stripe",
    description: "Connect your Stripe account to sync payments, subscriptions, and refunds.",
    fields: [
      {
        column: "api_key",
        label: "Secret key",
        placeholder: "sk_live_…",
        help: "Prefer a restricted key with read access to Balance, Charges, Checkout Sessions, Coupons, Disputes, Invoices, PaymentIntents, Refunds, and Subscriptions.",
        secret: true,
      },
    ],
  },
  shopify: {
    type: "shopify",
    label: "Shopify",
    description: "Connect your Shopify store to sync orders, refunds, and subscription contracts.",
    fields: [
      {
        column: "external_account_id",
        label: "Shop domain",
        placeholder: "example.myshopify.com",
        help: "Use the myshopify.com domain or full Shopify admin URL for the store.",
        secret: false,
      },
      {
        column: "api_key",
        label: "Admin access token",
        placeholder: "shpat_…",
        help: "Requires read_orders and read_own_subscription_contracts scopes.",
        secret: true,
      },
    ],
  },
  square: {
    type: "square",
    label: "Square",
    description:
      "Connect your Square account to sync payments, refunds, subscriptions, and invoices.",
    fields: [
      {
        column: "api_key",
        label: "Access token",
        help: "Requires PAYMENTS_READ, SUBSCRIPTIONS_READ, INVOICES_READ, MERCHANT_PROFILE_READ, and ITEMS_READ.",
        secret: true,
      },
    ],
  },
  lemon_squeezy: {
    type: "lemon_squeezy",
    label: "Lemon Squeezy",
    description: "Sync subscriptions and subscription invoices from your Lemon Squeezy store.",
    fields: [
      {
        column: "api_key",
        label: "API key",
        placeholder: "eyJ0eXAiOi…",
        help: "Lemon Squeezy API keys are not scoped; use a live-mode key from Settings → API.",
        secret: true,
      },
    ],
  },
  polar: {
    type: "polar",
    label: "Polar",
    description: "Sync orders and subscriptions from your Polar organization.",
    fields: [
      {
        column: "api_key",
        label: "Organization access token",
        placeholder: "polar_oat_…",
        help: "Requires orders:read, subscriptions:read, and refunds:read scopes.",
        secret: true,
      },
    ],
  },
  dodo: {
    type: "dodo",
    label: "Dodo Payments",
    description: "Sync payments, subscriptions, and refunds from Dodo Payments.",
    fields: [
      {
        column: "api_key",
        label: "API key",
        help: "Create a read-only key (leave Enable write access unchecked).",
        secret: true,
      },
    ],
  },
  google_search_console: {
    type: "google_search_console",
    label: "Search Console",
    description:
      "Connect Google Search Console with a service account to attribute organic search traffic.",
    fields: [
      {
        column: "external_account_id",
        label: "Site / property URL",
        placeholder: "sc-domain:example.com",
        help: "The verified Search Console property to read from. Add the service account as a user on this property.",
        secret: false,
      },
      {
        column: "api_key",
        label: "Service account email",
        placeholder: "revscope-gsc@example.iam.gserviceaccount.com",
        help: "Needs Search Console API enabled and Full or Restricted access on the property (read-only is enough).",
        secret: true,
      },
      {
        column: "secret",
        label: "Service account private key",
        placeholder: "-----BEGIN PRIVATE KEY-----…",
        secret: true,
      },
    ],
  },
  cloudflare: {
    type: "cloudflare",
    label: "Cloudflare",
    description:
      "Sync AI crawler request counts from Cloudflare GraphQL analytics. RevScope matches zones to the apex domains of hostnames in your traffic. Free plans expose the last 24 hours; RevScope keeps the history.",
    fields: [
      {
        column: "api_key",
        label: "API token",
        help: "Create a token with Zone → Zone → Read and Zone → Analytics → Read. Include every zone that serves your site.",
        secret: true,
      },
    ],
  },
};

/** An integration as returned by the API; credentials are masked. */
export type IntegrationView = {
  integration_type: IntegrationType;
  external_account_id: string | null;
  api_key_masked: string | null;
  secret_masked: string | null;
  /** ISO timestamp of the last successful sync with the current credentials and account, or null if none yet. */
  last_synced_at: string | null;
  /** Error from the most recent sync attempt with the current credentials and account, or null on success, if none yet, or while it runs. */
  last_sync_error: string | null;
  /** Error from the most recent such attempt that finished, which a running sync doesn't hide. Read through finishedSyncError. */
  last_finished_sync_error?: string | null;
};

/** The error to show as an integration's status. An older server only sends `last_sync_error`. */
export function finishedSyncError(
  integration: Pick<IntegrationView, "last_sync_error" | "last_finished_sync_error">,
): string | null {
  return integration.last_finished_sync_error ?? integration.last_sync_error;
}

export type SettingsResponse = {
  timezone: string;
  payment_success_path: string | null;
  custom_domain: string | null;
  integrations: IntegrationView[];
};

/**
 * Credential semantics for `api_key`/`secret`: `null` keeps the stored value,
 * `""` clears it, any other string replaces it.
 */
export type IntegrationSavePayload = {
  integration_type: IntegrationType;
  external_account_id: string | null;
  api_key: string | null;
  secret: string | null;
};

export type SaveSettingsPayload = {
  timezone: string;
  payment_success_path: string | null;
  custom_domain: string | null;
  integrations: IntegrationSavePayload[];
};

export async function fetchSettings(
  session: AuthSession,
  projectId: string,
): Promise<SettingsResponse> {
  const url = new URL(projectApiPath("session", projectId, "settings"), getApiBaseUrl());
  return fetchJson<SettingsResponse>(url, authRequestInit(session), "Unable to load settings.");
}

export async function saveSettings(
  session: AuthSession,
  projectId: string,
  payload: SaveSettingsPayload,
): Promise<SettingsResponse> {
  const url = new URL(projectApiPath("session", projectId, "settings"), getApiBaseUrl());
  return fetchJson<SettingsResponse>(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
    "Unable to save settings.",
  );
}

export type RevokeMcpTokensResponse = {
  revoked: number;
};

export async function revokeMcpTokens(session: AuthSession): Promise<RevokeMcpTokensResponse> {
  const url = new URL(sessionApiPath("mcp/revoke"), getApiBaseUrl());
  return fetchJson<RevokeMcpTokensResponse>(
    url,
    authRequestInit(session, { method: "POST" }),
    "Unable to revoke MCP tokens.",
  );
}

export type TimezoneOption = {
  /** IANA zone id, the value stored in user settings. */
  value: string;
  /** e.g. "(UTC-04:00) New York, Toronto". */
  label: string;
};

/**
 * Curated picker list: one zone per distinct UTC offset and DST rule, named
 * after the places most people recognise it by, west to east. Offsets aren't
 * baked in because they move with DST; they're computed at runtime.
 */
const TIMEZONE_PLACES: [zone: string, places: string][] = [
  ["Pacific/Pago_Pago", "Pago Pago, Midway"],
  ["Pacific/Honolulu", "Honolulu"],
  ["America/Anchorage", "Anchorage"],
  ["America/Los_Angeles", "Los Angeles, Vancouver"],
  ["America/Phoenix", "Phoenix"],
  ["America/Denver", "Denver, Calgary"],
  ["America/Mexico_City", "Mexico City"],
  ["America/Chicago", "Chicago, Winnipeg"],
  ["America/Bogota", "Bogotá, Lima"],
  ["America/New_York", "New York, Toronto"],
  ["America/Puerto_Rico", "San Juan, Caracas"],
  ["America/Halifax", "Halifax"],
  ["America/Santiago", "Santiago"],
  ["America/St_Johns", "St. John's"],
  ["America/Sao_Paulo", "São Paulo"],
  ["America/Argentina/Buenos_Aires", "Buenos Aires, Montevideo"],
  ["Atlantic/South_Georgia", "South Georgia"],
  ["Atlantic/Azores", "Azores"],
  ["Atlantic/Cape_Verde", "Cabo Verde"],
  ["UTC", "Coordinated Universal Time"],
  ["Africa/Abidjan", "Abidjan, Accra"],
  ["Europe/London", "London, Dublin"],
  ["Africa/Lagos", "Lagos, Algiers"],
  ["Europe/Paris", "Paris, Berlin"],
  ["Africa/Johannesburg", "Johannesburg, Harare"],
  ["Europe/Athens", "Athens, Helsinki"],
  ["Africa/Cairo", "Cairo"],
  ["Asia/Jerusalem", "Jerusalem"],
  ["Africa/Nairobi", "Nairobi, Addis Ababa"],
  ["Asia/Riyadh", "Riyadh, Baghdad"],
  ["Europe/Istanbul", "Istanbul"],
  ["Europe/Moscow", "Moscow"],
  ["Asia/Tehran", "Tehran"],
  ["Asia/Dubai", "Dubai, Baku"],
  ["Asia/Kabul", "Kabul"],
  ["Asia/Karachi", "Karachi, Tashkent"],
  ["Asia/Yekaterinburg", "Yekaterinburg"],
  ["Asia/Kolkata", "Mumbai, New Delhi"],
  ["Asia/Kathmandu", "Kathmandu"],
  ["Asia/Dhaka", "Dhaka, Almaty"],
  ["Asia/Yangon", "Yangon"],
  ["Asia/Bangkok", "Bangkok, Jakarta"],
  ["Asia/Shanghai", "Beijing, Shanghai"],
  ["Asia/Hong_Kong", "Hong Kong, Taipei"],
  ["Asia/Singapore", "Singapore, Kuala Lumpur"],
  ["Australia/Perth", "Perth"],
  ["Asia/Tokyo", "Tokyo, Osaka"],
  ["Asia/Seoul", "Seoul"],
  ["Australia/Darwin", "Darwin"],
  ["Australia/Adelaide", "Adelaide"],
  ["Australia/Brisbane", "Brisbane"],
  ["Australia/Sydney", "Sydney, Melbourne"],
  ["Asia/Vladivostok", "Vladivostok"],
  ["Pacific/Guadalcanal", "Honiara, Nouméa"],
  ["Pacific/Fiji", "Suva"],
  ["Pacific/Auckland", "Auckland, Wellington"],
  ["Pacific/Apia", "Apia"],
  ["Pacific/Kiritimati", "Kiritimati"],
];

/**
 * Curated zones ordered by their current UTC offset. `selected` is appended if
 * it isn't one of them, so a zone saved elsewhere still shows in the picker.
 */
export function timezoneOptions(selected?: string, at = new Date()): TimezoneOption[] {
  const places: [string, string][] = [...TIMEZONE_PLACES];
  if (selected && !places.some(([zone]) => zone === selected)) {
    places.push([selected, selected]);
  }
  return places
    .map(([zone, description]) => {
      const minutes = zoneOffsetMinutes(zone, at);
      return {
        value: zone,
        label: minutes === null ? zone : `(${formatUtcOffset(minutes)}) ${description}`,
        // Unknown zones sort last rather than pretending to sit at UTC.
        minutes: minutes ?? Number.POSITIVE_INFINITY,
      };
    })
    .sort((left, right) => left.minutes - right.minutes)
    .map(({ value, label }) => ({ value, label }));
}

/** Minutes east of UTC that `zone` observes at `at`, or null if it's unknown. */
function zoneOffsetMinutes(zone: string, at: Date): number | null {
  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).formatToParts(at);
  } catch {
    return null;
  }
  const part = (type: Intl.DateTimeFormatPartTypes) =>
    Number(parts.find((entry) => entry.type === type)?.value);
  const localAsUtc = Date.UTC(
    part("year"),
    part("month") - 1,
    part("day"),
    part("hour"),
    part("minute"),
    part("second"),
  );
  if (Number.isNaN(localAsUtc)) {
    return null;
  }
  // The formatted parts drop milliseconds; offsets are whole minutes anyway.
  return Math.round((localAsUtc - at.getTime()) / 60000);
}

function formatUtcOffset(minutes: number): string {
  const absolute = Math.abs(minutes);
  const hours = String(Math.floor(absolute / 60)).padStart(2, "0");
  const rest = String(absolute % 60).padStart(2, "0");
  return `UTC${minutes < 0 ? "-" : "+"}${hours}:${rest}`;
}
