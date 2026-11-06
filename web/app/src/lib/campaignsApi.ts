import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { ConversionInterval } from "./dashboardApi";
import type { MetricChange } from "./metricChange";
import { appendSegmentParams, type SegmentFilters } from "./segmentFilters";

export type CampaignRule = {
  id?: number;
  param_key: string;
  param_value: string;
};

export type CampaignCount = {
  label: string;
  visitors: number;
};

export type CampaignPathCount = {
  id: number;
  path: string;
  count: number;
};

export type CampaignListItem = {
  id: number;
  source: string;
  medium: string;
  campaign: string;
  network?: string;
  display_name?: string;
  first_seen_at: number;
  rules: CampaignRule[];
  visitors: number;
  converting_visitors: number;
  revenue: number;
  conversion_rate: number;
  /** 95% Wilson interval for the conversion rate; null without visitors. */
  conversion_ci: ConversionInterval | null;
  revenue_per_visitor: number;
  /** Entered spend on finished days within the range, in thousandths. */
  spend: number;
  /** Revenue ÷ spend; null without spend. */
  roas: number | null;
  /** (Revenue − spend) ÷ spend; null without spend. */
  roi: number | null;
  spend_entries: CampaignSpendEntry[];
  visitors_change: MetricChange;
  revenue_change: MetricChange;
  conversion_rate_change: MetricChange;
  referrers: CampaignCount[];
  landing_paths: CampaignPathCount[];
  countries: CampaignCount[];
  devices: CampaignCount[];
  contents: CampaignCount[];
  terms: CampaignCount[];
};

/** `once` is a single amount on start_date; `daily` is charged every day through end_date. */
export type CampaignSpendKind = "once" | "daily";

export type CampaignSpendInput = {
  kind: CampaignSpendKind;
  /** `YYYY-MM-DD` in the project timezone. */
  start_date: string;
  /** Inclusive; only used for `daily`. */
  end_date?: string;
  /** Thousandths of the project currency; per day for `daily`. */
  amount: number;
};

export type CampaignSpendEntry = Required<CampaignSpendInput> & {
  id: number;
  /** Spent on finished days, regardless of the selected period. */
  spent_to_date: number;
};

export type CampaignSuggestion = {
  /** Query parameter key: `source` or `ref`. */
  kind: string;
  value: string;
  visitors: number;
};

export type CampaignsResponse = {
  campaigns: CampaignListItem[];
  suggestions: CampaignSuggestion[];
};

export type CampaignPreviewResponse = {
  unclassified: number;
  direct: number;
  referrer: number;
  already_classified: number;
};

export type SaveCampaignRequest = {
  id?: number;
  name: string;
  rules: CampaignRule[];
};

export type SaveCampaignResponse = {
  id: number;
  assigned: number;
};

function campaignsUrl(
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters,
  resource: string,
): URL {
  const url = new URL(projectApiPath("read", projectId, resource), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  appendSegmentParams(url, segments);
  return url;
}

export async function fetchCampaigns(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters,
): Promise<CampaignsResponse> {
  return fetchJson(
    campaignsUrl(projectId, start, end, segments, "campaigns"),
    authRequestInit(session),
    "Unable to load campaigns.",
  );
}

export function previewCampaign(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters,
  rules: CampaignRule[],
): Promise<CampaignPreviewResponse> {
  return fetchJson(
    campaignsUrl(projectId, start, end, segments, "campaign-preview"),
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ rules }),
    }),
    "Unable to preview this campaign.",
  );
}

export async function saveCampaign(
  session: AuthSession,
  projectId: string,
  request: SaveCampaignRequest,
): Promise<SaveCampaignResponse> {
  const url = new URL(projectApiPath("session", projectId, "campaigns"), getApiBaseUrl());
  return fetchJson(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    }),
    "Unable to save this campaign.",
  );
}

/** Replaces every spend entry on the campaign. */
export async function saveCampaignSpend(
  session: AuthSession,
  projectId: string,
  campaignId: number,
  entries: CampaignSpendInput[],
): Promise<void> {
  const url = new URL(projectApiPath("session", projectId, "campaign-spend"), getApiBaseUrl());
  await fetchJson(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ campaign_id: campaignId, entries }),
    }),
    "Unable to save spend.",
  );
}

export async function deleteCampaign(
  session: AuthSession,
  projectId: string,
  id: number,
): Promise<void> {
  const url = new URL(projectApiPath("session", projectId, "campaigns"), getApiBaseUrl());
  url.searchParams.set("id", String(id));
  await fetchJson(
    url,
    authRequestInit(session, { method: "DELETE" }),
    "Unable to delete this campaign.",
  );
}
