import { Show } from "solid-js";
import {
  formatPercentRatio,
  formatRevenueAmount,
  formatSessionDuration,
} from "../../lib/dashboardApi";
import type { PageRow } from "../../lib/pagesApi";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import {
  clickableRow,
  gridFilterActionCell,
  gridFilterButton,
  gridFilterButtonActive,
  gridLandingPathsCell,
  gridLandingPathsText,
  gridNumberCell,
  gridRowHighlighted,
} from "./grid.css";
import HighlightedValue from "./HighlightedValue";
import { gridFilterIcon } from "./icons.css";
import MetricHoverCard from "./MetricHoverCard";
import { pageRowKey, pageUrl } from "./PagesTreemap";
import { buildPageHoverContent, type PagesVariant } from "./pageHoverDetails";
import { pageRowHighlights } from "./pagesLogic";

interface PageTableRowProps {
  row: PageRow;
  variant: PagesVariant;
  highlighted: boolean;
  expanded: boolean;
  onToggle: (row: PageRow) => void;
  onPageFilter?: (pathId: number, path: string) => void;
  activePageFilterKeys?: Set<string>;
}

/** One page's row in the pages grid, with a hover card of its metrics. */
export default function PageTableRow(props: PageTableRowProps) {
  const highlights = () => pageRowHighlights(props.row, props.variant);
  const drillDownEnabled = () => props.row.visitors > 0;
  const rowClass = () =>
    [
      drillDownEnabled() ? clickableRow : undefined,
      props.highlighted ? gridRowHighlighted : undefined,
    ]
      .filter(Boolean)
      .join(" ");
  const hoverContent = () => buildPageHoverContent(props.row, props.variant, highlights());

  const cells = (
    <>
      <td class={gridLandingPathsCell} title={pageUrl(props.row)}>
        <span class={gridLandingPathsText}>{pageUrl(props.row)}</span>
      </td>
      <td class={gridNumberCell}>
        <HighlightedValue positive={props.row.best_visitors_uplift}>
          {props.row.visitors.toLocaleString()}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>{formatRevenueAmount(props.row.attributed_revenue)}</td>
      <td class={gridNumberCell}>
        <HighlightedValue positive={props.row.best_conversion_uplift}>
          {formatWilsonScoreInterval(props.row.conversion_ci, 2)}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>
        <HighlightedValue
          positive={
            props.variant === "landingPages"
              ? props.row.best_bounce_uplift
              : props.row.best_exit_uplift
          }
          negative={
            props.variant === "landingPages"
              ? props.row.highest_bounce_rate
              : props.row.highest_exit_rate
          }
        >
          {formatPercentRatio(
            props.variant === "landingPages" ? props.row.bounce_rate : props.row.exit_rate,
            1,
          )}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>
        <HighlightedValue positive={props.row.best_session_uplift}>
          {formatSessionDuration(
            props.variant === "landingPages"
              ? props.row.avg_session_duration
              : props.row.avg_page_duration,
          )}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>{formatPercentRatio(props.row.avg_scroll_percent / 100, 1)}</td>
      <Show when={props.onPageFilter}>
        {(onPageFilter) => {
          const active = () => props.activePageFilterKeys?.has(String(props.row.id)) ?? false;
          const label = () =>
            active() ? `Filtering by ${pageUrl(props.row)}` : `Filter by ${pageUrl(props.row)}`;
          return (
            <td class={gridFilterActionCell}>
              <button
                type="button"
                class={
                  active() ? `${gridFilterButton} ${gridFilterButtonActive}` : gridFilterButton
                }
                title={label()}
                aria-label={label()}
                aria-pressed={active()}
                onClick={(event) => {
                  event.stopPropagation();
                  onPageFilter()(props.row.id, pageUrl(props.row));
                }}
              >
                <span class={gridFilterIcon} aria-hidden="true" />
              </button>
            </td>
          );
        }}
      </Show>
    </>
  );

  const rowTriggerProps = (
    mergeProps: (props: Record<string, unknown>) => Record<string, unknown>,
  ) => {
    const base = {
      "data-record-key": pageRowKey(props.row),
      class: rowClass(),
    };
    if (!drillDownEnabled()) {
      return mergeProps(base);
    }
    return mergeProps({
      ...base,
      role: "button",
      tabIndex: 0,
      "aria-expanded": props.expanded,
      "aria-label": `${props.expanded ? "Hide" : "Show"} transitions for ${pageUrl(props.row)}`,
      onClick: () => props.onToggle(props.row),
      onKeyDown: (event: KeyboardEvent) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          props.onToggle(props.row);
        }
      },
    });
  };

  return (
    <MetricHoverCard
      title={pageUrl(props.row)}
      content={hoverContent()}
      statLayout="twoColumn"
      size="revenueSource"
      showPeriodComparisonNote={false}
      renderTrigger={(mergeProps) => <tr {...rowTriggerProps(mergeProps)}>{cells}</tr>}
    />
  );
}
