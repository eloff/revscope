/// <reference types="solid-js" />
import type { AuthSession } from "@revscope/web-shared/authState";
import type { ProjectOption } from "@revscope/web-shared/projectSelection";
import { createResource, Show } from "solid-js";
import { fetchOverview, formatRevenueAmount, type OverviewResponse } from "../../lib/dashboardApi";
import {
  type CacheEntry,
  readFreshCacheEntry,
  writeCacheEntry,
} from "../../lib/dashboardDataCache";
import type { DateRangeSeconds } from "../../lib/dateRange";
import { createDefaultChartVisibility } from "./chartMetrics";
import MetricTile from "./MetricTile";
import OverviewChart from "./OverviewChart";
import {
  card,
  cardEmpty,
  cardHeader,
  cardOpen,
  cardOpenIcon,
  cardTiles,
  cardTitle,
  currentCard,
} from "./ProjectsOverview.css";
import { hasOverviewData } from "./tabDataPresence";

const CARD_CHART_HEIGHT = 260;
/** Visitors as the line and revenue as the bars, whatever the viewer toggled on Overview. */
const CARD_CHART_VISIBILITY = createDefaultChartVisibility();

export type ProjectOverviewCache = Map<string, CacheEntry<OverviewResponse>>;

type ProjectOverviewCardProps = {
  session: AuthSession;
  project: ProjectOption;
  range: DateRangeSeconds;
  current: boolean;
  cache: ProjectOverviewCache;
  onOpen: () => void;
};

export default function ProjectOverviewCard(props: ProjectOverviewCardProps) {
  const [overview] = createResource(
    () => ({ projectId: props.project.value, start: props.range.start, end: props.range.end }),
    async ({ projectId, start, end }) => {
      const key = `${projectId}:${start}:${end}`;
      const cached = readFreshCacheEntry(props.cache, key);
      if (cached) {
        return cached;
      }
      // Segment filters name rows of the selected project, so they don't apply here.
      const data = await fetchOverview(props.session, projectId, start, end);
      writeCacheEntry(props.cache, key, data);
      return data;
    },
  );
  const loading = () => overview.loading && !overview.latest;
  const error = () =>
    overview.error instanceof Error
      ? overview.error.message
      : overview.error
        ? "Unable to load."
        : "";
  const data = () => (overview.error ? undefined : overview.latest);

  return (
    <article class={props.current ? `${card} ${currentCard}` : card}>
      <button
        type="button"
        class={cardHeader}
        onClick={() => props.onOpen()}
        title={`Open ${props.project.label}`}
      >
        <span class={cardTitle}>{props.project.label}</span>
        <span class={cardOpen} aria-hidden="true">
          Open
          <svg class={cardOpenIcon} viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M6 3.5 10.5 8 6 12.5"
              stroke="currentColor"
              stroke-width="1.75"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </span>
      </button>
      <div class={cardTiles}>
        <MetricTile
          label="Visitors"
          value={(data()?.current_metrics.visitors ?? 0).toLocaleString()}
          change={data()?.changes.visitors}
          loading={loading()}
        />
        <MetricTile
          label="Revenue"
          value={formatRevenueAmount(data()?.current_metrics.total_revenue ?? 0)}
          change={data()?.changes.total_revenue}
          loading={loading()}
        />
      </div>
      <Show
        when={loading() || error() || (data() && hasOverviewData(data() as OverviewResponse))}
        fallback={<p class={cardEmpty}>No visitors or revenue in this period.</p>}
      >
        <OverviewChart
          data={data()}
          loading={loading()}
          error={error()}
          visibility={CARD_CHART_VISIBILITY}
          timeZone={props.project.timezone}
          rangeEnd={props.range.end}
          height={CARD_CHART_HEIGHT}
        />
      </Show>
    </article>
  );
}
