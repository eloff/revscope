import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import type { PaymentActivityResponse } from "../../lib/paymentActivityApi";
import {
  addCivilDays,
  civilDateKey,
  civilWeekday,
  compareCivilDates,
  zonedDate,
} from "../../lib/timezone";
import {
  activityCard,
  activityContent,
  activityHeader,
  activitySubtitle,
  activityTitle,
  categoryCheckboxInput,
  categoryCheckboxRow,
  categoryToggleColumn,
  dayCell,
  dayCellEmpty,
  graphHost,
  graphLayout,
  graphScroll,
  graphWithSidebar,
  gridBody,
  hoverActivityTable,
  hoverActivityTableBreakdownStart,
  hoverActivityTableCorner,
  hoverActivityTableMetricHeader,
  hoverActivityTableRowLabel,
  hoverActivityTableValue,
  hoverBreakdownLabel,
  legendRow,
  legendSwatch,
  legendSwatches,
  monthLabel,
  monthRow,
  toggleRow,
  weekColumn,
  weekdayColumn,
  weekdayLabel,
  weeksRow,
} from "./ActivityGraph.css";
import CursorHoverCard from "./CursorHoverCard";
import { createCursorHover } from "./createCursorHover";
import { useDashboardState } from "./dashboardPeriodContext";
import { hoverCardTitle } from "./hoverCard.css";
import { errorMessage, message } from "./messages.css";
import SegmentedControl from "./SegmentedControl";

type ActivityGraphProps = {
  data?: PaymentActivityResponse;
  loading: boolean;
  error: string;
};

type ActivityCategory = "single" | "new_subscription" | "recurring";
type ShadeBy = "revenue" | "payments";

const SHADE_OPTIONS: { value: ShadeBy; label: string }[] = [
  { value: "revenue", label: "Revenue" },
  { value: "payments", label: "Purchases" },
];

const CATEGORY_META: {
  key: ActivityCategory;
  label: string;
}[] = [
  { key: "single", label: "New revenue" },
  { key: "new_subscription", label: "New subscriptions" },
  { key: "recurring", label: "Recurring revenue" },
];

/** Ten-step (decile) green intensity scale, light to dark. */
const LEVEL_COLORS = [
  "#dcfce7",
  "#bbf7d0",
  "#86efac",
  "#4ade80",
  "#22c55e",
  "#16a34a",
  "#15803d",
  "#166534",
  "#14532d",
  "#0b3d23",
];

const EMPTY_COLOR = "#eef2f5";
const FUTURE_COLOR = "transparent";

const WEEKS = 53;
const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];
const WEEKDAY_LABELS = ["", "Mon", "", "Wed", "", "Fri", ""];
/** Width of one week column plus the inter-column gap, in pixels. */
const COLUMN_STRIDE = 18;

type DayCell = {
  date: string;
  count: number;
  revenue: number;
  single: number;
  single_revenue: number;
  new_subscription: number;
  new_subscription_revenue: number;
  recurring: number;
  recurring_revenue: number;
  /** 0 = no payments, 1..10 = decile level, -1 = outside data window / future. */
  level: number;
};

type GraphModel = {
  weeks: DayCell[][];
  monthLabels: { col: number; text: string }[];
  total: number;
  totalRevenue: number;
};

function ordinalSuffix(day: number): string {
  if (day >= 11 && day <= 13) {
    return "th";
  }
  switch (day % 10) {
    case 1:
      return "st";
    case 2:
      return "nd";
    case 3:
      return "rd";
    default:
      return "th";
  }
}

/**
 * `dateKey` is already a calendar day in the project's zone, so it is read back
 * as UTC purely to name the weekday and month.
 */
function formatActivityDate(dateKey: string): string {
  const [year, month, dayStr] = dateKey.split("-").map(Number);
  const date = new Date(Date.UTC(year, month - 1, dayStr));
  const weekday = date.toLocaleDateString(undefined, {
    weekday: "short",
    timeZone: "UTC",
  });
  const monthLabel = date.toLocaleDateString(undefined, {
    month: "short",
    timeZone: "UTC",
  });
  const day = date.getUTCDate();
  return `${weekday} ${monthLabel} ${day}${ordinalSuffix(day)}, ${year}`;
}

/** Linear-interpolated quantile over an ascending-sorted array. */
function quantile(sorted: number[], q: number): number {
  if (sorted.length === 0) {
    return 0;
  }
  const position = (sorted.length - 1) * q;
  const base = Math.floor(position);
  const rest = position - base;
  const lower = sorted[base];
  const upper = sorted[Math.min(base + 1, sorted.length - 1)];
  return lower + (upper - lower) * rest;
}

/** Decile thresholds (p10..p90) used to bucket a day's value into 1..10. */
function decileThresholds(positives: number[]): number[] {
  const thresholds: number[] = [];
  for (let percentile = 10; percentile <= 90; percentile += 10) {
    thresholds.push(quantile(positives, percentile / 100));
  }
  return thresholds;
}

function levelFor(value: number, thresholds: number[]): number {
  if (value <= 0) {
    return 0;
  }
  let level = 1;
  for (const threshold of thresholds) {
    if (value > threshold) {
      level += 1;
    }
  }
  return Math.min(level, 10);
}

function cellColor(level: number): string {
  if (level < 0) {
    return FUTURE_COLOR;
  }
  if (level === 0) {
    return EMPTY_COLOR;
  }
  return LEVEL_COLORS[level - 1];
}

function categoryBreakdown(
  cell: DayCell,
  category: ActivityCategory,
): { count: number; revenue: number } {
  switch (category) {
    case "single":
      return { count: cell.single, revenue: cell.single_revenue };
    case "new_subscription":
      return {
        count: cell.new_subscription,
        revenue: cell.new_subscription_revenue,
      };
    case "recurring":
      return { count: cell.recurring, revenue: cell.recurring_revenue };
  }
}

function ActivityDayHoverCardContent(props: {
  cell: DayCell;
  enabled: Record<ActivityCategory, boolean>;
}) {
  const breakdownRows = () =>
    CATEGORY_META.filter((meta) => {
      if (!props.enabled[meta.key]) {
        return false;
      }
      const breakdown = categoryBreakdown(props.cell, meta.key);
      return breakdown.count > 0 || breakdown.revenue > 0;
    });

  return (
    <>
      <div class={hoverCardTitle}>{formatActivityDate(props.cell.date)}</div>
      <table class={hoverActivityTable}>
        <thead>
          <tr>
            <th class={hoverActivityTableCorner} scope="col" />
            <th class={hoverActivityTableMetricHeader} scope="col">
              Revenue
            </th>
            <th class={hoverActivityTableMetricHeader} scope="col">
              Purchases
            </th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <th class={hoverActivityTableRowLabel} scope="row">
              Total
            </th>
            <td class={hoverActivityTableValue}>{formatRevenueAmount(props.cell.revenue)}</td>
            <td class={hoverActivityTableValue}>{props.cell.count.toLocaleString()}</td>
          </tr>
        </tbody>
        <Show when={breakdownRows().length > 0}>
          <tbody>
            <For each={breakdownRows()}>
              {(meta, index) => {
                const breakdown = () => categoryBreakdown(props.cell, meta.key);
                const breakdownCellClass = (base: string) =>
                  index() === 0 ? `${base} ${hoverActivityTableBreakdownStart}` : base;
                return (
                  <tr>
                    <th class={breakdownCellClass(hoverActivityTableRowLabel)} scope="row">
                      <span class={hoverBreakdownLabel}>{meta.label}</span>
                    </th>
                    <td class={breakdownCellClass(hoverActivityTableValue)}>
                      {formatRevenueAmount(breakdown().revenue)}
                    </td>
                    <td class={breakdownCellClass(hoverActivityTableValue)}>
                      {breakdown().count.toLocaleString()}
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </Show>
      </table>
    </>
  );
}

export default function ActivityGraph(props: ActivityGraphProps) {
  const { timeZone } = useDashboardState();
  const [enabled, setEnabled] = createSignal<Record<ActivityCategory, boolean>>({
    single: true,
    new_subscription: true,
    recurring: true,
  });
  const [shadeBy, setShadeBy] = createSignal<ShadeBy>("payments");
  const hover = createCursorHover<DayCell>();

  let graphScrollEl!: HTMLDivElement;

  const scrollGraphToEnd = () => {
    if (!graphScrollEl) {
      return;
    }
    graphScrollEl.scrollLeft = graphScrollEl.scrollWidth - graphScrollEl.clientWidth;
  };

  const model = createMemo<GraphModel | null>(() => {
    const response = props.data;
    if (!response) {
      return null;
    }
    const flags = enabled();
    const metric = shadeBy();
    const counts = new Map<string, number>();
    const revenues = new Map<string, number>();
    const dayData = new Map<
      string,
      Pick<
        DayCell,
        | "single"
        | "single_revenue"
        | "new_subscription"
        | "new_subscription_revenue"
        | "recurring"
        | "recurring_revenue"
      >
    >();
    let total = 0;
    let totalRevenue = 0;
    for (const day of response.days) {
      let value = 0;
      let revenue = 0;
      if (flags.single) {
        value += day.single;
        revenue += day.single_revenue;
      }
      if (flags.new_subscription) {
        value += day.new_subscription;
        revenue += day.new_subscription_revenue;
      }
      if (flags.recurring) {
        value += day.recurring;
        revenue += day.recurring_revenue;
      }
      counts.set(day.date, value);
      revenues.set(day.date, revenue);
      dayData.set(day.date, {
        single: day.single,
        single_revenue: day.single_revenue,
        new_subscription: day.new_subscription,
        new_subscription_revenue: day.new_subscription_revenue,
        recurring: day.recurring,
        recurring_revenue: day.recurring_revenue,
      });
      total += value;
      totalRevenue += revenue;
    }

    const shadeSource = metric === "revenue" ? revenues : counts;
    const positives = [...shadeSource.values()].filter((value) => value > 0).sort((a, b) => a - b);
    const thresholds = decileThresholds(positives);

    // `end` is exclusive, so the last day the grid can fill is the one holding
    // the final second of the window in the project's zone. Everything below is
    // civil arithmetic on that day, matching the calendar-day keys the API sends.
    const lastDay = zonedDate(timeZone(), response.end - 1);
    const endOfWeek = addCivilDays(lastDay, 6 - civilWeekday(lastDay));
    const startDay = addCivilDays(endOfWeek, -(WEEKS * 7 - 1));

    const weeks: DayCell[][] = [];
    const monthLabels: { col: number; text: string }[] = [];
    let lastMonth = -1;

    for (let week = 0; week < WEEKS; week += 1) {
      const column: DayCell[] = [];
      for (let row = 0; row < 7; row += 1) {
        const date = addCivilDays(startDay, week * 7 + row);
        const key = civilDateKey(date);
        const isFuture = compareCivilDates(date, lastDay) > 0;
        const count = counts.get(key) ?? 0;
        const revenue = revenues.get(key) ?? 0;
        const breakdown = dayData.get(key) ?? {
          single: 0,
          single_revenue: 0,
          new_subscription: 0,
          new_subscription_revenue: 0,
          recurring: 0,
          recurring_revenue: 0,
        };
        const shadeValue = metric === "revenue" ? revenue : count;
        column.push({
          date: key,
          count,
          revenue,
          ...breakdown,
          level: isFuture ? -1 : levelFor(shadeValue, thresholds),
        });
        if (row === 0) {
          const month = date.month - 1;
          if (month !== lastMonth) {
            monthLabels.push({ col: week, text: MONTH_NAMES[month] });
            lastMonth = month;
          }
        }
      }
      weeks.push(column);
    }

    return { weeks, monthLabels, total, totalRevenue };
  });

  const selectedSummary = createMemo(() => {
    const current = model();
    if (!current) {
      return "";
    }
    const active = CATEGORY_META.filter((meta) => enabled()[meta.key]);
    if (active.length === 0) {
      return "Select a payment type to see activity.";
    }
    if (shadeBy() === "revenue") {
      return `${formatRevenueAmount(current.totalRevenue)} in the last 12 months`;
    }
    return `${current.total.toLocaleString()} purchases in the last 12 months`;
  });

  createEffect(() => {
    if (!model()) {
      return;
    }
    queueMicrotask(scrollGraphToEnd);

    const el = graphScrollEl;
    if (!el) {
      return;
    }

    const observer = new ResizeObserver(scrollGraphToEnd);
    observer.observe(el);
    onCleanup(() => observer.disconnect());
  });

  const handleCellHover = (cell: DayCell, clientX: number, clientY: number) => {
    if (cell.level < 0) {
      hover.clear();
      return;
    }
    hover.show(cell, clientX, clientY);
  };

  return (
    <div class={activityCard}>
      <div class={activityContent}>
        <div class={activityHeader}>
          <div>
            <h3 class={activityTitle}>Purchase activity</h3>
            <p class={activitySubtitle}>{selectedSummary()}</p>
          </div>
          <div class={toggleRow}>
            <SegmentedControl value={shadeBy()} options={SHADE_OPTIONS} onChange={setShadeBy} />
          </div>
        </div>

        <Show when={props.loading}>
          <p class={message}>Loading purchase activity...</p>
        </Show>
        <Show when={props.error}>
          <p class={errorMessage}>{props.error}</p>
        </Show>

        <Show when={model()}>
          {(graph) => (
            <div class={graphWithSidebar}>
              <div class={graphScroll} ref={graphScrollEl}>
                <div class={`${graphLayout} ${graphHost}`}>
                  <div class={monthRow}>
                    <For each={graph().monthLabels.slice(1)}>
                      {(label) => (
                        <span class={monthLabel} style={{ left: `${label.col * COLUMN_STRIDE}px` }}>
                          {label.text}
                        </span>
                      )}
                    </For>
                  </div>
                  <div class={gridBody}>
                    <div class={weekdayColumn}>
                      <For each={WEEKDAY_LABELS}>
                        {(label) => <span class={weekdayLabel}>{label}</span>}
                      </For>
                    </div>
                    {/* biome-ignore lint/a11y/noStaticElementInteractions: hover-only activity tooltip */}
                    <div class={weeksRow} onMouseLeave={hover.clear}>
                      <For each={graph().weeks}>
                        {(week) => (
                          <div class={weekColumn}>
                            <For each={week}>
                              {(cell) => (
                                // biome-ignore lint/a11y/noStaticElementInteractions: hover-only activity tooltip
                                <div
                                  class={cell.level < 0 ? `${dayCell} ${dayCellEmpty}` : dayCell}
                                  style={{
                                    background: cellColor(cell.level),
                                  }}
                                  onMouseEnter={(event) =>
                                    handleCellHover(cell, event.clientX, event.clientY)
                                  }
                                  onMouseMove={(event) =>
                                    handleCellHover(cell, event.clientX, event.clientY)
                                  }
                                />
                              )}
                            </For>
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                  <div class={legendRow}>
                    <span>Less</span>
                    <div class={legendSwatches}>
                      <span class={legendSwatch} style={{ background: EMPTY_COLOR }} />
                      <For each={LEVEL_COLORS}>
                        {(color) => <span class={legendSwatch} style={{ background: color }} />}
                      </For>
                    </div>
                    <span>More</span>
                  </div>
                </div>
                <CursorHoverCard hover={hover} open={hover.key() !== null} closeDelay={100}>
                  <Show when={hover.key()}>
                    {(cell) => <ActivityDayHoverCardContent cell={cell()} enabled={enabled()} />}
                  </Show>
                </CursorHoverCard>
              </div>
              <div class={categoryToggleColumn}>
                <For each={CATEGORY_META}>
                  {(meta) => (
                    <label class={categoryCheckboxRow}>
                      <input
                        class={categoryCheckboxInput}
                        type="checkbox"
                        checked={enabled()[meta.key]}
                        onChange={(event) => {
                          setEnabled((previous) => ({
                            ...previous,
                            [meta.key]: event.currentTarget.checked,
                          }));
                        }}
                      />
                      {meta.label}
                    </label>
                  )}
                </For>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
