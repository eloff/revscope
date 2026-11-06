import type { CampaignListItem } from "../../lib/campaignsApi";

/** Query parameters a campaign rule can match, as stored by the tracker. */
export const RULE_PARAM_KEYS = ["source", "ref", "medium", "content", "term"] as const;

const PARAM_LABELS: Record<string, string> = {
  source: "utm_source",
  medium: "utm_medium",
  campaign: "utm_campaign",
  content: "utm_content",
  term: "utm_term",
  ref: "ref",
};

export function paramLabel(key: string): string {
  return PARAM_LABELS[key] ?? key;
}

export function campaignTitle(campaign: CampaignListItem): string {
  if (campaign.display_name) {
    return campaign.display_name;
  }
  if (campaign.campaign) {
    return campaign.campaign;
  }
  if (campaign.network && campaign.network !== "generic") {
    return `${campaign.network} ads`;
  }
  return "Unnamed campaign";
}

export function campaignSubtitle(campaign: CampaignListItem): string {
  if (campaign.rules.length > 0) {
    return "Custom rule";
  }
  const parts = [campaign.source, campaign.medium].filter((part) => part !== "");
  if (campaign.network && campaign.network !== "generic") {
    parts.unshift(`${campaign.network} click id`);
  }
  return parts.length > 0 ? parts.join(" · ") : "UTM campaign";
}
