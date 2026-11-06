/// <reference types="solid-js" />
import { Tabs } from "@ark-ui/solid/tabs";
import { createSignal, For, onMount } from "solid-js";
import type { OverviewBreakdowns as OverviewBreakdownsData } from "../../lib/dashboardApi";
import { ensureWorldFlagsSpriteStylesheet } from "../../lib/worldFlagsSprite";
import BreakdownTable from "./BreakdownTable";
import { breakdownPanel, nestedTabTrigger } from "./breakdownTabs.css";
import { breakdownGrid, breakdownHeader, breakdownTabList } from "./OverviewBreakdowns.css";
import {
  BREAKDOWN_PANELS,
  type BreakdownKey,
  type BreakdownPanelConfig,
} from "./overviewBreakdownsLogic";

type OverviewBreakdownsProps = {
  /** Undefined before the first load. */
  data: OverviewBreakdownsData | undefined;
  loading: boolean;
};

/**
 * Compact visitors-and-revenue panels for every filterable dimension. Clicking
 * a row toggles it as a global segment filter.
 */
export default function OverviewBreakdowns(props: OverviewBreakdownsProps) {
  onMount(ensureWorldFlagsSpriteStylesheet);
  return (
    <div class={breakdownGrid}>
      <For each={BREAKDOWN_PANELS}>
        {(panel) => <BreakdownPanel panel={panel} data={props.data} loading={props.loading} />}
      </For>
    </div>
  );
}

function BreakdownPanel(props: {
  panel: BreakdownPanelConfig;
  data: OverviewBreakdownsData | undefined;
  loading: boolean;
}) {
  const [tab, setTab] = createSignal<BreakdownKey>(props.panel.tabs[0].key);
  return (
    <Tabs.Root
      value={tab()}
      onValueChange={(details) => setTab(details.value as BreakdownKey)}
      class={breakdownPanel}
      aria-label={props.panel.title}
      // Tables of tabs never opened aren't rendered; each can hold up to 200
      // rows, and every overview reload rebuilds them.
      lazyMount
    >
      <div class={breakdownHeader}>
        <Tabs.List class={breakdownTabList}>
          <For each={props.panel.tabs}>
            {(item) => (
              <Tabs.Trigger value={item.key} class={nestedTabTrigger}>
                {item.label}
              </Tabs.Trigger>
            )}
          </For>
        </Tabs.List>
      </div>
      <For each={props.panel.tabs}>
        {(item) => (
          <Tabs.Content value={item.key}>
            <BreakdownTable tab={item} rows={props.data?.[item.key]} loading={props.loading} />
          </Tabs.Content>
        )}
      </For>
    </Tabs.Root>
  );
}
