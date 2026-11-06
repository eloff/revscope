import { Tabs } from "@ark-ui/solid/tabs";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { type AudienceResponse, CITIES_PAGE_SIZE } from "../../lib/audienceApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import AudienceBreakdownTreemap from "./AudienceBreakdownTreemap";
import AudienceHoverCardContent from "./AudienceHoverCardContent";
import AudienceMetricTable, {
  AUDIENCE_VALUE_COLUMNS,
  type AudienceRowFilter,
  COUNTRY_VALUE_COLUMNS,
} from "./AudienceMetricTable";
import {
  type AudienceBreakdownHighlights,
  type AudienceBreakdownRow,
  buildCityBreakdownRows,
  buildCityHoverContent,
  buildCityMetrics,
  type CityMetrics,
  findAudienceBreakdownHighlights,
  isIsoCountryCode,
  withAudienceBreakdownHighlightDescription,
} from "./audienceLogic";
import {
  breakdownPanel,
  nestedTabContent,
  nestedTabList,
  nestedTabTrigger,
} from "./breakdownTabs.css";
import CountryFlag from "./CountryFlag";
import CursorHoverCard from "./CursorHoverCard";
import { type CursorHover, createCursorHover } from "./createCursorHover";
import { createRowHighlight } from "./createRowHighlight";
import { useDashboardState } from "./dashboardPeriodContext";
import { gridIconLabelCell } from "./grid.css";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";

const GEO_DIMENSIONS = [
  { value: "country", label: "Country" },
  { value: "cities", label: "Cities" },
] as const;

type GeoDimension = (typeof GEO_DIMENSIONS)[number]["value"];

interface AudienceGeoBreakdownProps {
  data: AudienceResponse | undefined;
  /** Cities ranked by revenue (offset 0), used only for the cities treemap. */
  treemapData: AudienceResponse | undefined;
  /** True while a request is in flight, including refreshes of data already shown. */
  loading: boolean;
  /** True until the first data arrives; the tables show a loading row meanwhile. */
  initialLoading: boolean;
  treemapLoading: boolean;
  countryRows: AudienceBreakdownRow[];
  countryHighlights: AudienceBreakdownHighlights;
  /** Shared with the map, which shows the same country hover card. */
  countryHover: CursorHover;
  scrollTarget?: { key: string; id: number } | null;
  onCountrySelect?: (countryCode: string) => void;
  onCountryFilter?: (countryCode: string, countryName: string) => void;
  citiesOffset?: number;
  citiesLoading?: boolean;
  onCitiesPageChange?: (offset: number) => void;
  onCitiesSortChange?: () => void;
}

function cityHoverTitle(metrics: CityMetrics): string {
  if (metrics.countryName) {
    return `${metrics.city}, ${metrics.countryName}`;
  }
  return metrics.city;
}

/** Country and city breakdowns: a treemap over a table, per tab. */
export default function AudienceGeoBreakdown(props: AudienceGeoBreakdownProps) {
  const { segments } = useDashboardState();
  const { audienceBreakdownSorts, setAudienceBreakdownSort } = useDashboardPreferences();
  const [dimension, setDimension] = createSignal<GeoDimension>("country");
  const [countryTbody, setCountryTbody] = createSignal<HTMLTableSectionElement>();
  const { highlightKey: highlightedCountry, scrollToRow } = createRowHighlight();
  const cityHover = createCursorHover();

  const countryRowFilter = createMemo((): AudienceRowFilter | undefined => {
    const onCountryFilter = props.onCountryFilter;
    if (!onCountryFilter) {
      return undefined;
    }
    const active = new Set(segments().countries.map((country) => country.code));
    return {
      canFilter: (row) => isIsoCountryCode(row.key),
      isActive: (row) => active.has(row.key),
      onFilter: (row) => onCountryFilter(row.key, row.name),
    };
  });
  const cityRows = createMemo(() => (props.data ? buildCityBreakdownRows(props.data.cities) : []));
  const treemapCityRows = createMemo(() =>
    props.treemapData ? buildCityBreakdownRows(props.treemapData.cities) : [],
  );
  const cityHighlights = createMemo(() => {
    if ((props.citiesOffset ?? 0) > 0) {
      return {};
    }
    return findAudienceBreakdownHighlights(cityRows(), ["revenue"]);
  });
  const cityMetrics = createMemo<Map<string, CityMetrics>>(() => {
    const map = props.data ? buildCityMetrics(props.data.cities) : new Map<string, CityMetrics>();
    if (props.treemapData && props.treemapData !== props.data) {
      for (const [key, metrics] of buildCityMetrics(props.treemapData.cities)) {
        map.set(key, metrics);
      }
    }
    return map;
  });
  const hoveredCity = createMemo(() => {
    const key = cityHover.key();
    return key ? (cityMetrics().get(key) ?? null) : null;
  });
  const activeHover = () => (dimension() === "country" ? props.countryHover : cityHover);

  createEffect(() => {
    const target = props.scrollTarget;
    if (target?.key && isIsoCountryCode(target.key)) {
      setDimension("country");
    }
  });

  createEffect(() => {
    const target = props.scrollTarget;
    const tbody = countryTbody();
    if (!target || !tbody || props.loading) {
      return;
    }
    scrollToRow(tbody, target.key);
  });

  return (
    <Tabs.Root
      value={dimension()}
      onValueChange={(details) => setDimension(details.value as GeoDimension)}
      class={breakdownPanel}
    >
      <Tabs.List class={nestedTabList}>
        <For each={GEO_DIMENSIONS}>
          {(tab) => (
            <Tabs.Trigger value={tab.value} class={nestedTabTrigger}>
              {tab.label}
            </Tabs.Trigger>
          )}
        </For>
      </Tabs.List>
      <AudienceBreakdownTreemap
        rows={dimension() === "country" ? props.countryRows : treemapCityRows()}
        topGroupColor={
          dimension() === "country" ? SOURCE_GROUP_COLORS.social : SOURCE_GROUP_COLORS.referrers
        }
        otherTooltipNoun={dimension() === "country" ? "countries" : "cities"}
        loading={
          props.initialLoading ||
          (dimension() === "cities" && props.treemapLoading && !props.treemapData)
        }
        onLeafSelect={(key) => {
          if (dimension() === "country" && isIsoCountryCode(key)) {
            props.onCountrySelect?.(key);
          }
        }}
        onLeafHover={(key, clientX, clientY) => activeHover().show(key, clientX, clientY)}
        onChartMouseMove={(clientX, clientY) => activeHover().move(clientX, clientY)}
      />
      <Tabs.Content value="country" class={nestedTabContent}>
        <AudienceMetricTable
          rows={props.countryRows}
          columns={COUNTRY_VALUE_COLUMNS}
          highlights={props.countryHighlights}
          sort={audienceBreakdownSorts().country}
          loading={props.initialLoading}
          nameColumnLabel="Country"
          countSingularLabel="country"
          countPluralLabel="countries"
          emptyLabel="No countries for this period"
          showCountryFlags
          highlightedKey={highlightedCountry()}
          tbodyRef={setCountryTbody}
          rowFilter={countryRowFilter()}
          onSort={(sort) => setAudienceBreakdownSort("country", sort)}
          onRowHover={props.countryHover.show}
          onRowLeave={props.countryHover.clear}
        />
      </Tabs.Content>
      <Tabs.Content value="cities" class={nestedTabContent}>
        <CursorHoverCard hover={cityHover} open={hoveredCity() !== null}>
          <Show when={hoveredCity()}>
            {(city) => (
              <AudienceHoverCardContent
                title={
                  <span class={gridIconLabelCell}>
                    <Show
                      when={(() => {
                        const code = city().countryCode;
                        return code && isIsoCountryCode(code) ? code : undefined;
                      })()}
                    >
                      {(code) => <CountryFlag code={code()} />}
                    </Show>
                    <span>{cityHoverTitle(city())}</span>
                  </span>
                }
                detail={withAudienceBreakdownHighlightDescription(
                  buildCityHoverContent(city()),
                  city().key,
                  cityHighlights(),
                )}
              />
            )}
          </Show>
        </CursorHoverCard>
        <AudienceMetricTable
          rows={cityRows()}
          columns={AUDIENCE_VALUE_COLUMNS}
          highlights={cityHighlights()}
          sort={audienceBreakdownSorts().cities}
          loading={props.initialLoading || (props.citiesLoading ?? false)}
          nameColumnLabel="City"
          countSingularLabel="city"
          countPluralLabel="cities"
          emptyLabel="No cities for this period"
          serverSorted
          offset={props.citiesOffset ?? props.data?.cities.offset ?? 0}
          limit={props.data?.cities.limit ?? CITIES_PAGE_SIZE}
          nextOffset={props.citiesLoading ? undefined : props.data?.cities.next_offset}
          onPageChange={props.onCitiesPageChange}
          onSort={(sort) => {
            setAudienceBreakdownSort("cities", sort);
            props.onCitiesSortChange?.();
          }}
          onRowHover={cityHover.show}
          onRowLeave={cityHover.clear}
        />
      </Tabs.Content>
    </Tabs.Root>
  );
}
