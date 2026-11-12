/// <reference types="solid-js" />

import type { AuthSession } from "@revscope/web-shared/authState";
import { createEffect, createSignal, For, Show } from "solid-js";
import {
  type CampaignListItem,
  type CampaignPreviewResponse,
  type CampaignRule,
  type CampaignSpendInput,
  type CampaignSuggestion,
  type CampaignsResponse,
  deleteCampaign,
  fetchCampaigns,
  previewCampaign,
  saveCampaign,
  saveCampaignSpend,
} from "../../lib/campaignsApi";
import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type { DateRangeSeconds } from "../../lib/dateRange";
import {
  hasSegmentFilters,
  type SegmentFilters,
  segmentFiltersKey,
} from "../../lib/segmentFilters";
import { primaryButton } from "../buttons.css";
import CampaignDetails from "./CampaignDetails";
import CampaignEditor, {
  type CampaignDraft,
  draftRules,
  emptyRule,
  rulesFromCampaign,
} from "./CampaignEditor";
import {
  campaignNameText,
  campaignSubtitle as campaignSubtitleClass,
  campaignsDescription,
  campaignsHeader,
  campaignsShell,
  campaignsTitle,
  chevron,
  chevronButton,
  chevronCell,
  countPill,
  dateCell,
  metricHeader,
  metricHeaderWithIcon,
  mutedText,
  nameCell,
  paramKey,
  paramValue,
  placeholderValue,
  suggestionChip,
  suggestionList,
  suggestionsCard,
  suggestionsTitle,
  tableScroller,
  textHeader,
} from "./Campaigns.css";
import { campaignSubtitle, campaignTitle, paramLabel } from "./campaignLabels";
import { formatRoas } from "./campaignSpendLogic";
import { formatSourceConversionCell } from "./conversionConfidence";
import { useDashboardState } from "./dashboardPeriodContext";
import GridColumnInfoIcon from "./GridColumnInfoIcon";
import {
  clickableRow,
  gridExpandedCell,
  gridExpandedRow,
  gridNumberCell,
  gridTable,
} from "./grid.css";
import { errorMessage } from "./messages.css";
import { campaignDisplayLabel } from "./revenueSourcesLogic";

const COLUMN_COUNT = 9;

type CampaignsProps = {
  projectId: string;
  publicAccess?: boolean;
  session?: AuthSession;
};

function formatFirstSeen(timestamp: number): string {
  if (!timestamp) {
    return "—";
  }
  // Ingest stores nanoseconds. Older generated rows store unix seconds.
  const millis = timestamp > 1_000_000_000_000_000 ? timestamp / 1_000_000 : timestamp * 1000;
  return new Date(millis).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function SpendFilterWarning(props: { metric: "ROAS" | "ROI" }) {
  return (
    <GridColumnInfoIcon
      tone="warning"
      title={`${props.metric} is skewed by dashboard filters`}
      description={`Dashboard filters narrow each campaign's revenue, but spend is entered for the whole campaign and isn't filtered, so ${props.metric} here comes out lower than it really is. Remove the dashboard filters to see an accurate ${props.metric}.`}
    />
  );
}

export default function Campaigns(props: CampaignsProps) {
  const { range, rangeReady, segments, addSourceFilter } = useDashboardState();
  const [data, setData] = createSignal<CampaignsResponse>();
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const [expandedId, setExpandedId] = createSignal<number | null>(null);
  const [draft, setDraft] = createSignal<CampaignDraft | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [actionError, setActionError] = createSignal("");
  let requestToken = 0;

  const canEdit = () => !props.publicAccess && Boolean(props.session);
  // Campaign rows ignore the source filter, so only the other dimensions narrow revenue.
  const filtersActive = () => hasSegmentFilters({ ...segments(), sources: [] });
  const creating = () => {
    const current = draft();
    return current !== null && current.id === undefined;
  };

  // The demo reads anonymously, since a signed-in viewer is not its owner.
  const readSession = () => (props.publicAccess ? undefined : props.session);

  const load = async (current: DateRangeSeconds, currentSegments: SegmentFilters) => {
    if (!rangeReady() || !(props.publicAccess || props.session)) {
      return;
    }
    const token = ++requestToken;
    setLoading(true);
    setError("");
    try {
      const response = await fetchCampaigns(
        readSession(),
        props.projectId,
        current.start,
        current.end,
        currentSegments,
      );
      if (token === requestToken) {
        setData(response);
      }
    } catch (loadError) {
      if (token === requestToken) {
        setError(loadError instanceof Error ? loadError.message : "Unable to load campaigns.");
      }
    } finally {
      if (token === requestToken) {
        setLoading(false);
      }
    }
  };

  createEffect(() => {
    const current = range();
    const currentSegments = segments();
    segmentFiltersKey(currentSegments);
    if (!rangeReady()) {
      return;
    }
    void load(current, currentSegments);
  });

  const reload = () => load(range(), segments());

  const runPreview = (rules: CampaignRule[]): Promise<CampaignPreviewResponse> => {
    const current = range();
    const currentSegments = segments();
    return previewCampaign(
      readSession(),
      props.projectId,
      current.start,
      current.end,
      currentSegments,
      rules,
    );
  };

  const openDraft = (next: CampaignDraft) => {
    setActionError("");
    setDraft(next);
  };

  const closeDraft = () => {
    setActionError("");
    setDraft(null);
  };

  const saveDraft = async () => {
    const current = draft();
    const session = props.session;
    if (!current || !session || !canEdit()) {
      return;
    }
    const rules = current.renameOnly ? [] : draftRules(current.rules);
    if (!current.name.trim()) {
      setActionError("Give the campaign a name.");
      return;
    }
    if (!current.renameOnly && rules.length === 0) {
      setActionError("Add a value for at least one condition.");
      return;
    }
    setSaving(true);
    setActionError("");
    try {
      await saveCampaign(session, props.projectId, {
        id: current.id,
        name: current.name.trim(),
        rules,
      });
      setDraft(null);
      await reload();
    } catch (saveError) {
      setActionError(saveError instanceof Error ? saveError.message : "Unable to save.");
    } finally {
      setSaving(false);
    }
  };

  const removeCampaign = async (campaign: CampaignListItem) => {
    const session = props.session;
    if (!session) {
      return;
    }
    const name = campaignTitle(campaign);
    const confirmed = window.confirm(
      `Delete ${name}? It's removed from Campaigns and from the Campaigns group in Sources. ` +
        "Its visits will count under their referrer or as direct.",
    );
    if (!confirmed) {
      return;
    }
    setActionError("");
    try {
      await deleteCampaign(session, props.projectId, campaign.id);
      if (expandedId() === campaign.id) {
        setExpandedId(null);
      }
      await reload();
    } catch (deleteError) {
      setActionError(deleteError instanceof Error ? deleteError.message : "Unable to delete.");
    }
  };

  const saveSpend = async (campaign: CampaignListItem, entries: CampaignSpendInput[]) => {
    const session = props.session;
    if (!session || !canEdit()) {
      return;
    }
    await saveCampaignSpend(session, props.projectId, campaign.id, entries);
    await reload();
  };

  const trackSuggestion = (suggestion: CampaignSuggestion) => {
    openDraft({
      name: suggestion.value.slice(0, 80),
      rules: [{ paramKey: suggestion.kind, paramValue: suggestion.value }],
    });
  };

  const editCampaign = (campaign: CampaignListItem) => {
    openDraft({
      id: campaign.id,
      name: campaignTitle(campaign),
      rules: rulesFromCampaign(campaign.rules),
      renameOnly: campaign.rules.length === 0,
    });
  };

  const toggleRow = (campaign: CampaignListItem) => {
    if (draft()?.id === campaign.id) {
      closeDraft();
    }
    setExpandedId((current) => (current === campaign.id ? null : campaign.id));
  };

  const editor = (current: () => CampaignDraft) => (
    <CampaignEditor
      draft={current()}
      saving={saving()}
      readOnly={!canEdit()}
      error={actionError()}
      onChange={setDraft}
      onCancel={closeDraft}
      onSave={() => void saveDraft()}
      previewCampaign={runPreview}
    />
  );

  return (
    <div class={campaignsShell}>
      <div class={campaignsHeader}>
        <div>
          <h2 class={campaignsTitle}>Campaigns</h2>
          <p class={campaignsDescription}>
            Visits tagged with <code>utm_campaign</code> or an ad click id show up here on their
            own. Create a campaign to group other tagged traffic, like{" "}
            <code>utm_source=betalist</code> or <code>ref=producthunt</code>.
          </p>
        </div>
        <Show when={!creating()}>
          <button
            type="button"
            class={primaryButton}
            onClick={() => openDraft({ name: "", rules: [emptyRule()] })}
          >
            + New campaign
          </button>
        </Show>
      </div>
      <Show when={error()}>
        <p class={errorMessage}>{error()}</p>
      </Show>
      <Show when={actionError() && !draft()}>
        <p class={errorMessage}>{actionError()}</p>
      </Show>
      <Show when={creating() && draft()}>{(current) => editor(current)}</Show>
      <Show when={!creating() && (data()?.suggestions.length ?? 0) > 0}>
        <section class={suggestionsCard}>
          <div>
            <h3 class={suggestionsTitle}>Untracked sources</h3>
            <p class={mutedText}>
              Frequent tags on visits without a campaign this period. Choose one to make it a
              campaign.
            </p>
          </div>
          <ul class={suggestionList}>
            <For each={data()?.suggestions ?? []}>
              {(suggestion) => (
                <li>
                  <button
                    type="button"
                    class={suggestionChip}
                    onClick={() => trackSuggestion(suggestion)}
                  >
                    <span class={paramKey}>{paramLabel(suggestion.kind)}=</span>
                    <span class={paramValue}>{suggestion.value}</span>
                    <span class={countPill} title="Visitors">
                      {suggestion.visitors.toLocaleString()}
                    </span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
      <div class={tableScroller}>
        <table class={gridTable}>
          <thead>
            <tr>
              <th class={chevronCell} aria-label="Expand" />
              <th>
                <span class={textHeader}>Campaign</span>
              </th>
              <th class={gridNumberCell}>
                <span class={metricHeader}>Visitors</span>
              </th>
              <th
                class={gridNumberCell}
                title="Spend entered for finished days in this period. Today is left out."
              >
                <span class={metricHeader}>Spend</span>
              </th>
              <th class={gridNumberCell}>
                <span class={metricHeader}>Revenue</span>
              </th>
              <th
                class={gridNumberCell}
                title={filtersActive() ? undefined : "Return on ad spend: revenue ÷ spend."}
              >
                <span class={metricHeaderWithIcon}>
                  ROAS
                  <Show when={filtersActive()}>
                    <SpendFilterWarning metric="ROAS" />
                  </Show>
                </span>
              </th>
              <th
                class={gridNumberCell}
                title={
                  filtersActive() ? undefined : "Return on investment: (revenue − spend) ÷ spend."
                }
              >
                <span class={metricHeaderWithIcon}>
                  ROI
                  <Show when={filtersActive()}>
                    <SpendFilterWarning metric="ROI" />
                  </Show>
                </span>
              </th>
              <th class={gridNumberCell}>
                <span class={metricHeaderWithIcon}>
                  Conversion rate
                  <GridColumnInfoIcon
                    title="Conversion rate"
                    description="Converting visitors divided by visitors, shown as a 95% Wilson confidence interval. Expand a campaign to see the exact rate."
                  />
                </span>
              </th>
              <th>
                <span class={textHeader}>First seen</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <For each={data()?.campaigns ?? []}>
              {(campaign) => {
                const open = () => expandedId() === campaign.id;
                return (
                  <>
                    <tr class={clickableRow} onClick={() => toggleRow(campaign)}>
                      <td class={chevronCell}>
                        <button
                          type="button"
                          class={chevronButton}
                          aria-expanded={open()}
                          aria-label={`${open() ? "Hide" : "Show"} details for ${campaignTitle(campaign)}`}
                        >
                          <span class={chevron} data-open={open()} aria-hidden="true">
                            ▶
                          </span>
                        </button>
                      </td>
                      <td class={nameCell}>
                        <span class={campaignNameText}>{campaignTitle(campaign)}</span>
                        <span class={campaignSubtitleClass}>{campaignSubtitle(campaign)}</span>
                      </td>
                      <td class={gridNumberCell}>{campaign.visitors.toLocaleString()}</td>
                      <td
                        class={gridNumberCell}
                        classList={{ [placeholderValue]: campaign.spend_entries.length === 0 }}
                      >
                        {campaign.spend_entries.length === 0
                          ? "—"
                          : formatRevenueAmount(campaign.spend)}
                      </td>
                      <td class={gridNumberCell}>{formatRevenueAmount(campaign.revenue)}</td>
                      <td
                        class={gridNumberCell}
                        classList={{ [placeholderValue]: campaign.roas === null }}
                      >
                        {formatRoas(campaign.roas)}
                      </td>
                      <td
                        class={gridNumberCell}
                        classList={{ [placeholderValue]: campaign.roi === null }}
                      >
                        {campaign.roi === null ? "—" : formatPercentRatio(campaign.roi, 0)}
                      </td>
                      <td class={gridNumberCell}>
                        {formatSourceConversionCell(
                          campaign.conversion_rate,
                          campaign.conversion_ci,
                        )}
                      </td>
                      <td class={dateCell}>{formatFirstSeen(campaign.first_seen_at)}</td>
                    </tr>
                    <Show when={open()}>
                      <tr class={gridExpandedRow}>
                        <td class={gridExpandedCell} colSpan={COLUMN_COUNT}>
                          <Show
                            when={draft()?.id === campaign.id && draft()}
                            fallback={
                              <CampaignDetails
                                campaign={campaign}
                                canEdit={canEdit()}
                                onFilter={() =>
                                  addSourceFilter({
                                    kind: "campaign",
                                    id: campaign.id,
                                    name: campaignDisplayLabel(campaign),
                                  })
                                }
                                onEdit={() => editCampaign(campaign)}
                                onDelete={() => void removeCampaign(campaign)}
                                onSaveSpend={(entries) => saveSpend(campaign, entries)}
                              />
                            }
                          >
                            {(current) => editor(current)}
                          </Show>
                        </td>
                      </tr>
                    </Show>
                  </>
                );
              }}
            </For>
            <Show when={loading() && !data()}>
              <tr>
                <td colSpan={COLUMN_COUNT}>
                  <span class={mutedText}>Loading campaigns…</span>
                </td>
              </tr>
            </Show>
            <Show when={!loading() && data() && (data()?.campaigns.length ?? 0) === 0}>
              <tr>
                <td colSpan={COLUMN_COUNT}>
                  <span class={mutedText}>
                    No campaign traffic in this period. Create a campaign to start grouping tagged
                    visits.
                  </span>
                </td>
              </tr>
            </Show>
          </tbody>
        </table>
      </div>
    </div>
  );
}
