import { Tabs } from "@ark-ui/solid/tabs";
import { createMemo, createSignal, For, Show } from "solid-js";
import type { AudienceResponse } from "../../lib/audienceApi";
import {
  type AudienceBreakdownPreferenceKey,
  useDashboardPreferences,
} from "../../lib/dashboardPreferences";
import {
  type AttributeFilter,
  attributeFilterKey,
  type LandingAttribute,
} from "../../lib/segmentFilters";
import AudienceBreakdownTreemap from "./AudienceBreakdownTreemap";
import AudienceHoverCardContent from "./AudienceHoverCardContent";
import AudienceMetricTable, {
  AUDIENCE_VALUE_COLUMNS,
  type AudienceRowFilter,
} from "./AudienceMetricTable";
import {
  type AudienceBreakdownRow,
  browserFamilyIconClass,
  buildBrowserBreakdownRows,
  buildBrowserMetrics,
  buildDeviceBreakdownRows,
  buildDeviceMetrics,
  buildOsBreakdownRows,
  buildOsMetrics,
  buildScreenSizeBreakdownRows,
  buildScreenSizeMetrics,
  buildTechHoverContent,
  buildViewportBreakdownRows,
  buildViewportMetrics,
  deviceIconClass,
  osIconClass,
  type TechMetrics,
} from "./audienceLogic";
import {
  breakdownPanel,
  nestedTabContent,
  nestedTabList,
  nestedTabTrigger,
} from "./breakdownTabs.css";
import CursorHoverCard from "./CursorHoverCard";
import { createCursorHover } from "./createCursorHover";
import { useDashboardState } from "./dashboardPeriodContext";
import { gridIconLabelCell } from "./grid.css";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";

type TechDimension = {
  value: string;
  label: string;
  sortKey: AudienceBreakdownPreferenceKey;
  nameColumnLabel: string;
  countSingularLabel: string;
  countPluralLabel: string;
  emptyLabel: string;
  /** Used in the treemap's "Other" tooltip, e.g. "outside the top 10 browsers". */
  otherTooltipNoun: string;
  iconClass?: (key: string) => string;
  /** The segment filter a row's key selects; screen and viewport sizes have none. */
  filterAttribute?: LandingAttribute;
  buildRows: (data: AudienceResponse) => AudienceBreakdownRow[];
  buildMetrics: (data: AudienceResponse) => Map<string, TechMetrics>;
};

const TECH_DIMENSIONS: TechDimension[] = [
  {
    value: "browser",
    label: "Browser",
    sortKey: "browser",
    nameColumnLabel: "Browser",
    countSingularLabel: "browser",
    countPluralLabel: "browsers",
    emptyLabel: "No browsers for this period",
    otherTooltipNoun: "browsers",
    iconClass: browserFamilyIconClass,
    filterAttribute: "browser",
    buildRows: (data) => buildBrowserBreakdownRows(data.browsers),
    buildMetrics: (data) => buildBrowserMetrics(data.browsers),
  },
  {
    value: "os",
    label: "OS",
    sortKey: "os",
    nameColumnLabel: "OS",
    countSingularLabel: "OS",
    countPluralLabel: "operating systems",
    emptyLabel: "No operating systems for this period",
    otherTooltipNoun: "operating systems",
    iconClass: osIconClass,
    filterAttribute: "os",
    buildRows: (data) => buildOsBreakdownRows(data.os),
    buildMetrics: (data) => buildOsMetrics(data.os),
  },
  {
    value: "device",
    label: "Device",
    sortKey: "device",
    nameColumnLabel: "Device",
    countSingularLabel: "device",
    countPluralLabel: "devices",
    emptyLabel: "No devices for this period",
    otherTooltipNoun: "devices",
    iconClass: deviceIconClass,
    filterAttribute: "device",
    buildRows: (data) => buildDeviceBreakdownRows(data.devices),
    buildMetrics: (data) => buildDeviceMetrics(data.devices),
  },
  {
    value: "screen_size",
    label: "Screen size",
    sortKey: "screenSize",
    nameColumnLabel: "Screen pixel width",
    countSingularLabel: "screen size bin",
    countPluralLabel: "screen size bins",
    emptyLabel: "No screen sizes for this period",
    otherTooltipNoun: "screen size bins",
    buildRows: (data) => buildScreenSizeBreakdownRows(data.screen_sizes),
    buildMetrics: (data) => buildScreenSizeMetrics(data.screen_sizes),
  },
  {
    value: "viewport",
    label: "Viewport size",
    sortKey: "viewport",
    nameColumnLabel: "Viewport pixel width",
    countSingularLabel: "viewport bin",
    countPluralLabel: "viewport bins",
    emptyLabel: "No viewports for this period",
    otherTooltipNoun: "viewport bins",
    buildRows: (data) => buildViewportBreakdownRows(data.viewports),
    buildMetrics: (data) => buildViewportMetrics(data.viewports),
  },
];

interface AudienceTechBreakdownProps {
  data: AudienceResponse | undefined;
  loading: boolean;
  onAttributeFilter?: (filter: AttributeFilter) => void;
}

/** Browser, OS, device, and screen breakdowns: a treemap over a table, per tab. */
export default function AudienceTechBreakdown(props: AudienceTechBreakdownProps) {
  const { audienceBreakdownSorts, setAudienceBreakdownSort } = useDashboardPreferences();
  const { segments } = useDashboardState();
  const rowFilterFor = (entry: TechDimension): AudienceRowFilter | undefined => {
    const attribute = entry.filterAttribute;
    const onAttributeFilter = props.onAttributeFilter;
    if (!attribute || !onAttributeFilter) {
      return undefined;
    }
    const filterFor = (row: { key: string; name: string }): AttributeFilter => ({
      attribute,
      value: row.key,
      name: row.name,
    });
    return {
      canFilter: () => true,
      isActive: (row) => {
        const key = attributeFilterKey(filterFor(row));
        return segments().attributes.some((filter) => attributeFilterKey(filter) === key);
      },
      onFilter: (row) => onAttributeFilter(filterFor(row)),
    };
  };
  const [dimension, setDimension] = createSignal(TECH_DIMENSIONS[0]);
  const hover = createCursorHover();

  const rowsByDimension = new Map(
    TECH_DIMENSIONS.map((entry) => [
      entry.value,
      createMemo(() => (props.data ? entry.buildRows(props.data) : [])),
    ]),
  );
  const rowsFor = (entry: TechDimension) => rowsByDimension.get(entry.value)?.() ?? [];
  const metrics = createMemo(() =>
    props.data ? dimension().buildMetrics(props.data) : new Map<string, TechMetrics>(),
  );
  const hoveredMetrics = createMemo(() => {
    const key = hover.key();
    return key ? (metrics().get(key) ?? null) : null;
  });

  return (
    <Tabs.Root
      value={dimension().value}
      onValueChange={(details) => {
        const next = TECH_DIMENSIONS.find((entry) => entry.value === details.value);
        if (next) {
          setDimension(next);
        }
        hover.clear();
      }}
      class={breakdownPanel}
    >
      <Tabs.List class={nestedTabList}>
        <For each={TECH_DIMENSIONS}>
          {(entry) => (
            <Tabs.Trigger value={entry.value} class={nestedTabTrigger}>
              {entry.label}
            </Tabs.Trigger>
          )}
        </For>
      </Tabs.List>
      <CursorHoverCard hover={hover} open={hoveredMetrics() !== null}>
        <Show when={hoveredMetrics()}>
          {(tech) => (
            <AudienceHoverCardContent
              title={
                <Show when={dimension().iconClass} fallback={<span>{tech().name}</span>}>
                  {(iconClass) => (
                    <span class={gridIconLabelCell}>
                      <span class={iconClass()(tech().key)} aria-hidden="true" />
                      <span>{tech().name}</span>
                    </span>
                  )}
                </Show>
              }
              detail={buildTechHoverContent(tech())}
            />
          )}
        </Show>
      </CursorHoverCard>
      <AudienceBreakdownTreemap
        rows={rowsFor(dimension())}
        topGroupColor={SOURCE_GROUP_COLORS.campaigns}
        otherTooltipNoun={dimension().otherTooltipNoun}
        loading={props.loading}
        onLeafHover={hover.show}
        onChartMouseMove={hover.move}
      />
      <For each={TECH_DIMENSIONS}>
        {(entry) => (
          <Tabs.Content value={entry.value} class={nestedTabContent}>
            <AudienceMetricTable
              rows={rowsFor(entry)}
              columns={AUDIENCE_VALUE_COLUMNS}
              sort={audienceBreakdownSorts()[entry.sortKey]}
              loading={props.loading}
              nameColumnLabel={entry.nameColumnLabel}
              countSingularLabel={entry.countSingularLabel}
              countPluralLabel={entry.countPluralLabel}
              emptyLabel={entry.emptyLabel}
              iconClass={entry.iconClass}
              rowFilter={rowFilterFor(entry)}
              onSort={(sort) => setAudienceBreakdownSort(entry.sortKey, sort)}
              onRowHover={hover.show}
              onRowLeave={hover.clear}
            />
          </Tabs.Content>
        )}
      </For>
    </Tabs.Root>
  );
}
