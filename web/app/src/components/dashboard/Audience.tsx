/// <reference types="solid-js" />
import { createMemo, For, onMount, Show } from "solid-js";
import type { AudienceResponse } from "../../lib/audienceApi";
import type { AttributeFilter } from "../../lib/segmentFilters";
import { ensureWorldFlagsSpriteStylesheet } from "../../lib/worldFlagsSprite";
import {
  audienceBreakdownGrid,
  audienceLegend,
  audienceLegendCaption,
  audienceLegendLabel,
  audienceLegendScale,
  audienceLegendSwatch,
  audienceMapWrapper,
  audiencePanel,
} from "./Audience.css";
import AudienceGeoBreakdown from "./AudienceGeoBreakdown";
import AudienceHoverCardContent from "./AudienceHoverCardContent";
import AudienceMap from "./AudienceMap";
import AudienceTechBreakdown from "./AudienceTechBreakdown";
import {
  bucketByRevenue,
  buildCountryBreakdownRows,
  buildCountryHoverContent,
  buildCountryMetrics,
  type CountryMetrics,
  findAudienceBreakdownHighlights,
  isIsoCountryCode,
  REVENUE_BUCKET_COLORS,
  withAudienceBreakdownHighlightDescription,
} from "./audienceLogic";
import CountryFlag from "./CountryFlag";
import CursorHoverCard from "./CursorHoverCard";
import { createCursorHover } from "./createCursorHover";
import { gridIconLabelCell } from "./grid.css";

export type AudienceProps = {
  data: AudienceResponse | undefined;
  /** Cities ranked by revenue (offset 0), used only for the cities treemap. */
  treemapData: AudienceResponse | undefined;
  loading: boolean;
  treemapLoading: boolean;
  error: string;
  scrollTarget?: { key: string; id: number } | null;
  onCountrySelect?: (countryCode: string) => void;
  /** Adds a global country segment filter from a country row's filter action. */
  onCountryFilter?: (countryCode: string, countryName: string) => void;
  /** Adds a global browser, OS, or device segment filter from a row's filter action. */
  onAttributeFilter?: (filter: AttributeFilter) => void;
  /** Requested cities page offset (server-side pagination). */
  citiesOffset?: number;
  /** True while a cities page/sort fetch is in flight and the table should show loading. */
  citiesLoading?: boolean;
  onCitiesPageChange?: (offset: number) => void;
  onCitiesSortChange?: () => void;
};

/** The audience tab: a revenue map of countries over geo and tech breakdowns. */
export default function Audience(props: AudienceProps) {
  onMount(() => {
    ensureWorldFlagsSpriteStylesheet();
  });

  // The map, the country treemap, and the country table share one hover card.
  const countryHover = createCursorHover();
  const countryMetrics = createMemo<Map<string, CountryMetrics>>(() =>
    props.data ? buildCountryMetrics(props.data.countries) : new Map(),
  );
  const buckets = createMemo(() => bucketByRevenue(countryMetrics()));
  const countryRows = createMemo(() =>
    props.data ? buildCountryBreakdownRows(props.data.countries) : [],
  );
  const countryHighlights = createMemo(() => findAudienceBreakdownHighlights(countryRows()));
  const hoveredCountry = createMemo(() => {
    const code = countryHover.key();
    return code ? (countryMetrics().get(code) ?? null) : null;
  });

  const showInitialLoading = () => props.loading && !props.data;

  return (
    <div class={audiencePanel}>
      <div class={audienceMapWrapper}>
        <AudienceMap
          buckets={buckets()}
          loading={showInitialLoading()}
          error={props.error}
          onHover={countryHover.show}
          onLeave={countryHover.clear}
          onRegionSelect={(code) => {
            if (isIsoCountryCode(code)) {
              props.onCountrySelect?.(code);
            }
          }}
        />
      </div>
      <CursorHoverCard hover={countryHover} open={hoveredCountry() !== null}>
        <Show when={hoveredCountry()}>
          {(country) => (
            <AudienceHoverCardContent
              title={
                <span class={gridIconLabelCell}>
                  <Show when={isIsoCountryCode(country().code)}>
                    <CountryFlag code={country().code} />
                  </Show>
                  <span>{country().name}</span>
                </span>
              }
              detail={withAudienceBreakdownHighlightDescription(
                buildCountryHoverContent(country()),
                country().code,
                countryHighlights(),
              )}
            />
          )}
        </Show>
      </CursorHoverCard>

      <div class={audienceLegend}>
        <span class={audienceLegendLabel}>Revenue share</span>
        <div class={audienceLegendScale}>
          <span class={audienceLegendCaption}>Less</span>
          <For each={REVENUE_BUCKET_COLORS}>
            {(color) => <span class={audienceLegendSwatch} style={{ background: color }} />}
          </For>
          <span class={audienceLegendCaption}>More</span>
        </div>
      </div>

      <div class={audienceBreakdownGrid}>
        <AudienceGeoBreakdown
          data={props.data}
          treemapData={props.treemapData}
          loading={props.loading}
          initialLoading={showInitialLoading()}
          treemapLoading={props.treemapLoading}
          countryRows={countryRows()}
          countryHighlights={countryHighlights()}
          countryHover={countryHover}
          scrollTarget={props.scrollTarget}
          onCountrySelect={props.onCountrySelect}
          onCountryFilter={props.onCountryFilter}
          citiesOffset={props.citiesOffset}
          citiesLoading={props.citiesLoading}
          onCitiesPageChange={props.onCitiesPageChange}
          onCitiesSortChange={props.onCitiesSortChange}
        />
        <AudienceTechBreakdown
          data={props.data}
          loading={showInitialLoading()}
          onAttributeFilter={props.onAttributeFilter}
        />
      </div>
    </div>
  );
}
