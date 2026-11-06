/// <reference types="solid-js" />
import { Toggle } from "@ark-ui/solid/toggle";
import { For, Show } from "solid-js";
import type { MetricsResponse, OverviewResponse, PeriodMetrics } from "../../lib/dashboardApi";
import {
  formatRevenueAmount,
  formatRevenueAmountFixed,
  formatSessionDuration,
} from "../../lib/dashboardApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import {
  type ChartMetricKey,
  isMetricChartActive,
  METRIC_CHART_COLORS,
  toggleChartMetric,
} from "./chartMetrics";
import MetricHoverCard from "./MetricHoverCard";
import { metricsBar, metricsBarScroll, metricTileToggle } from "./MetricsBar.css";
import MetricTile from "./MetricTile";
import { buildMetricHoverContent } from "./metricHoverDetails";

type MetricFormat = "currency" | "currencyFixed" | "count" | "percent" | "duration";

type MetricDefinition = {
  key: keyof PeriodMetrics;
  label: string;
  unit?: string;
  format: MetricFormat;
  percentDecimals?: number;
  higherIsBetter?: boolean;
  /** API stores some outflows as negative; show magnitude in the tile. */
  absValue?: boolean;
  /** When false, the tile does not toggle chart series. */
  chartable?: boolean;
};

const METRIC_DEFINITIONS: MetricDefinition[] = [
  { key: "total_revenue", label: "Revenue", format: "currency" },
  { key: "new_revenue", label: "New revenue", format: "currency" },
  { key: "mrr", label: "MRR", format: "currency" },
  {
    key: "new_subscriptions_mrr",
    label: "New MRR",
    format: "currency",
  },
  {
    key: "churn_mrr",
    label: "Churned MRR",
    format: "currency",
    higherIsBetter: false,
  },
  {
    key: "refunds_and_chargebacks",
    label: "Refunds",
    format: "currency",
    higherIsBetter: false,
    absValue: true,
  },
  { key: "visitors", label: "Visitors", format: "count" },
  {
    key: "revenue_per_visitor",
    label: "Revenue / visitor",
    format: "currencyFixed",
  },
  {
    key: "conversion_rate",
    label: "Conversion rate",
    format: "percent",
    percentDecimals: 3,
  },
  { key: "session_time", label: "Session time", format: "duration" },
  {
    key: "bounce_rate",
    label: "Bounce rate",
    format: "percent",
    higherIsBetter: false,
  },
];

const ALWAYS_VISIBLE_METRICS = new Set<keyof PeriodMetrics>(["total_revenue", "visitors"]);

function metricTileTooltip(label: string, onChart: boolean): string {
  return onChart ? `Hide ${label} from chart` : `Show ${label} on chart`;
}

function formatMetricValue(
  value: number,
  format: MetricFormat,
  percentDecimals = 1,
): { value: string; unit?: string } {
  switch (format) {
    case "currency":
      return { value: formatRevenueAmount(value) };
    case "currencyFixed":
      return { value: formatRevenueAmountFixed(value, 3) };
    case "count":
      return { value: value.toLocaleString() };
    case "percent":
      if (!Number.isFinite(value)) {
        return { value: "—" };
      }
      return { value: (value * 100).toFixed(percentDecimals), unit: "%" };
    case "duration":
      return { value: formatSessionDuration(value) };
  }
}

export type MetricsBarProps = {
  data: MetricsResponse | undefined;
  overview: OverviewResponse | undefined;
  loading?: boolean;
};

export default function MetricsBar(props: MetricsBarProps) {
  const { chartVisibility, setChartVisibility } = useDashboardPreferences();

  return (
    <Show when={props.loading || props.data}>
      <div class={metricsBar}>
        <div class={metricsBarScroll}>
          <For each={METRIC_DEFINITIONS}>
            {(definition) => {
              const metricValue = (period: "current" | "previous") => {
                const raw = props.data?.[period][definition.key] ?? 0;
                return definition.absValue ? Math.abs(raw) : raw;
              };
              const current = () => metricValue("current");
              const formatted = () =>
                formatMetricValue(current(), definition.format, definition.percentDecimals);
              const visible = () =>
                props.loading || ALWAYS_VISIBLE_METRICS.has(definition.key) || current() !== 0;
              const chartable = () => definition.chartable !== false;
              const chartKey = () => definition.key as ChartMetricKey;
              const pressed = () =>
                chartable() && isMetricChartActive(chartVisibility(), chartKey());
              const accentColor = () => (chartable() ? METRIC_CHART_COLORS[chartKey()] : undefined);
              const hoverContent = () => {
                const overview = props.overview;
                const metrics = props.data;
                if (!overview || !metrics) {
                  return undefined;
                }
                return buildMetricHoverContent(definition.key, overview, metrics.current);
              };

              const change = () => props.overview?.changes[definition.key];

              const tile = () => (
                <MetricTile
                  label={definition.label}
                  value={formatted().value}
                  unit={definition.unit ?? formatted().unit}
                  change={change()}
                  higherIsBetter={definition.higherIsBetter}
                  loading={props.loading}
                  accentColor={accentColor()}
                  active={pressed()}
                />
              );

              return (
                <Show when={visible()}>
                  <MetricHoverCard
                    title={definition.label}
                    content={hoverContent()}
                    loading={props.loading}
                    statLayout="threeColumn"
                    size="overview"
                    renderTrigger={(mergeProps) =>
                      chartable() ? (
                        <Toggle.Root
                          {...mergeProps({
                            class: metricTileToggle,
                            title: metricTileTooltip(definition.label, pressed()),
                          })}
                          pressed={pressed()}
                          onPressedChange={() => {
                            setChartVisibility(toggleChartMetric(chartVisibility(), chartKey()));
                          }}
                        >
                          {tile()}
                        </Toggle.Root>
                      ) : (
                        <div {...mergeProps({ class: metricTileToggle })}>{tile()}</div>
                      )
                    }
                  />
                </Show>
              );
            }}
          </For>
        </div>
      </div>
    </Show>
  );
}
