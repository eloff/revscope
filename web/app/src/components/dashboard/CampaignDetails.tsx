/// <reference types="solid-js" />
import { For, Show } from "solid-js";
import type { CampaignCount, CampaignListItem, CampaignSpendInput } from "../../lib/campaignsApi";
import { formatPercentRatio } from "../../lib/dashboardApi";
import CampaignSpend from "./CampaignSpend";
import {
  breakdownBar,
  breakdownCount,
  breakdownGrid,
  breakdownItem,
  breakdownLabel,
  breakdownList,
  conditionChip,
  conditionList,
  conditionOperator,
  dangerButton,
  detailsActions,
  detailsCard,
  detailsLayout,
  logicPill,
  mutedText,
  paramKey,
  paramValue,
  ruleNote,
  secondaryButton,
  sectionLabel,
  statItem,
  statList,
} from "./Campaigns.css";
import { paramLabel } from "./campaignLabels";
import { formatWilsonScoreInterval } from "./conversionConfidence";

type CampaignDetailsProps = {
  campaign: CampaignListItem;
  canEdit: boolean;
  onFilter: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onSaveSpend: (entries: CampaignSpendInput[]) => Promise<void>;
};

type Condition = { key: string; value: string };

function tagConditions(campaign: CampaignListItem): Condition[] {
  const conditions: Condition[] = [];
  if (campaign.network && campaign.network !== "generic") {
    conditions.push({ key: "ad click id", value: campaign.network });
  }
  for (const key of ["source", "medium", "campaign"] as const) {
    if (campaign[key] !== "") {
      conditions.push({ key: paramLabel(key), value: campaign[key] });
    }
  }
  return conditions;
}

export default function CampaignDetails(props: CampaignDetailsProps) {
  const hasRules = () => props.campaign.rules.length > 0;
  const conditions = (): Condition[] =>
    hasRules()
      ? props.campaign.rules.map((rule) => ({
          key: paramLabel(rule.param_key),
          value: rule.param_value,
        }))
      : tagConditions(props.campaign);
  const definitionLabel = () => {
    if (!hasRules()) {
      return "Matches visits tagged with";
    }
    return props.campaign.rules.length > 1
      ? "Matches visits where any of these is true"
      : "Matches visits where";
  };

  return (
    <div class={detailsLayout}>
      <section class={detailsCard}>
        <h3 class={sectionLabel}>{definitionLabel()}</h3>
        <div class={conditionList}>
          <For each={conditions()}>
            {(condition, index) => (
              <>
                <Show when={index() > 0}>
                  <span class={logicPill}>{hasRules() ? "OR" : "AND"}</span>
                </Show>
                <span class={conditionChip}>
                  <span class={paramKey}>{condition.key}</span>
                  <span class={conditionOperator}>=</span>
                  <span class={paramValue}>{condition.value}</span>
                </span>
              </>
            )}
          </For>
        </div>
        <Show when={hasRules()}>
          <p class={ruleNote}>Only matches visits not already part of a campaign.</p>
        </Show>
      </section>
      <section class={detailsCard}>
        <h3 class={sectionLabel}>Conversion</h3>
        <dl class={statList}>
          <div class={statItem}>
            <dt>Conversion rate</dt>
            <dd>
              {props.campaign.visitors > 0
                ? formatPercentRatio(props.campaign.conversion_rate, 2)
                : "—"}
            </dd>
          </div>
          <div class={statItem}>
            <dt>Converting visitors</dt>
            <dd>
              {props.campaign.converting_visitors.toLocaleString()} of{" "}
              {props.campaign.visitors.toLocaleString()}
            </dd>
          </div>
          <div class={statItem}>
            <dt>95% confidence interval</dt>
            <dd>{formatWilsonScoreInterval(props.campaign.conversion_ci, 2)}</dd>
          </div>
        </dl>
      </section>
      <CampaignSpend
        entries={props.campaign.spend_entries}
        canEdit={props.canEdit}
        onSave={props.onSaveSpend}
      />
      <div class={breakdownGrid}>
        <Breakdown title="Referrers" rows={props.campaign.referrers} />
        <Breakdown
          title="Landing pages"
          rows={props.campaign.landing_paths.map((entry) => ({
            label: entry.path,
            visitors: entry.count,
          }))}
        />
        <Breakdown title="Countries" rows={props.campaign.countries} />
        <Breakdown title="Devices" rows={props.campaign.devices} />
        <Show when={props.campaign.contents.length > 0}>
          <Breakdown title={paramLabel("content")} rows={props.campaign.contents} />
        </Show>
        <Show when={props.campaign.terms.length > 0}>
          <Breakdown title={paramLabel("term")} rows={props.campaign.terms} />
        </Show>
      </div>
      <div class={detailsActions}>
        <button type="button" class={secondaryButton} onClick={() => props.onFilter()}>
          Filter dashboard to this campaign
        </button>
        <Show when={props.canEdit}>
          <button type="button" class={secondaryButton} onClick={() => props.onEdit()}>
            {hasRules() ? "Edit conditions" : "Rename"}
          </button>
          <button type="button" class={dangerButton} onClick={() => props.onDelete()}>
            Delete campaign
          </button>
        </Show>
      </div>
    </div>
  );
}

function Breakdown(props: { title: string; rows: CampaignCount[] }) {
  const max = () => Math.max(1, ...props.rows.map((row) => row.visitors));
  return (
    <section class={detailsCard}>
      <h3 class={sectionLabel}>{props.title}</h3>
      <Show when={props.rows.length > 0} fallback={<p class={mutedText}>None in this period.</p>}>
        <ul class={breakdownList}>
          <For each={props.rows}>
            {(row) => (
              <li class={breakdownItem} title={row.label}>
                <span class={breakdownBar} style={{ width: `${(row.visitors / max()) * 100}%` }} />
                <span class={breakdownLabel}>{row.label}</span>
                <span class={breakdownCount}>{row.visitors.toLocaleString()}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
