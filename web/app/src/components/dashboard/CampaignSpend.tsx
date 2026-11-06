/// <reference types="solid-js" />
import { createSignal, For, Show } from "solid-js";
import type { CampaignSpendEntry, CampaignSpendInput } from "../../lib/campaignsApi";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import {
  detailsCard,
  mutedText,
  secondaryButton,
  sectionLabel,
  spendHeader,
  spendItem,
  spendList,
  spendToDate,
} from "./Campaigns.css";
import { formatSpendDate, spendRowsFromEntries } from "./campaignSpendLogic";
import SpendEditor from "./SpendEditor";

type CampaignSpendProps = {
  entries: CampaignSpendEntry[];
  canEdit: boolean;
  onSave: (entries: CampaignSpendInput[]) => Promise<void>;
};

function describeEntry(entry: CampaignSpendEntry): string {
  if (entry.kind === "once") {
    return `${formatRevenueAmount(entry.amount)} on ${formatSpendDate(entry.start_date)}`;
  }
  return `${formatRevenueAmount(entry.amount)}/day, ${formatSpendDate(entry.start_date)} – ${formatSpendDate(entry.end_date)}`;
}

export default function CampaignSpend(props: CampaignSpendProps) {
  const [editing, setEditing] = createSignal(false);

  return (
    <section class={detailsCard}>
      <div class={spendHeader}>
        <h3 class={sectionLabel}>Spend</h3>
        <Show when={!editing()}>
          <button type="button" class={secondaryButton} onClick={() => setEditing(true)}>
            {props.entries.length > 0 ? "Edit spend" : "Add spend"}
          </button>
        </Show>
      </div>
      <Show
        when={editing()}
        fallback={
          <Show
            when={props.entries.length > 0}
            fallback={
              <p class={mutedText}>
                No spend entered. Add it to see ROAS and ROI for this campaign.
              </p>
            }
          >
            <ul class={spendList}>
              <For each={props.entries}>
                {(entry) => (
                  <li class={spendItem}>
                    <span>{describeEntry(entry)}</span>
                    <span class={spendToDate} title="Spent on finished days, any period">
                      {formatRevenueAmount(entry.spent_to_date)} to date
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        }
      >
        <SpendEditor
          initialRows={spendRowsFromEntries(props.entries)}
          readOnly={!props.canEdit}
          onCancel={() => setEditing(false)}
          onSave={async (entries) => {
            await props.onSave(entries);
            setEditing(false);
          }}
        />
      </Show>
    </section>
  );
}
