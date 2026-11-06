/// <reference types="solid-js" />
import { DatePicker, parseDate } from "@ark-ui/solid/date-picker";
import LoggedInUser from "@revscope/web-shared/LoggedInUser";
import { appPath } from "@revscope/web-shared/links";
import type { ProjectOption } from "@revscope/web-shared/projectSelection";
// import SegmentedControl from "./SegmentedControl";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Index,
  onMount,
  Show,
  untrack,
} from "solid-js";
import revscopeLogo from "../../assets/revscope-logo.webp";
import {
  clampRangeToMaxDays,
  DATE_SHORTCUT_OPTIONS,
  type DateRangeSeconds,
  type DateShortcut,
  type DateValueLike,
  DEFAULT_DATE_SHORTCUT,
  dateValueToUnixEndExclusive,
  dateValueToUnixStart,
  nowUnixSeconds,
  rangeDayCount,
  shiftRange as shiftDateRange,
  shortcutForRange,
  shortcutToRange,
  unixStartToDateValue,
} from "../../lib/dateRange";
import {
  attributeFilterKey,
  eventFilterKey,
  hasSegmentFilters,
  LANDING_ATTRIBUTE_LABELS,
  landingPageFilterKey,
  pageFilterKey,
  sourceFilterKey,
} from "../../lib/segmentFilters";
import { formatTimeZoneLabel } from "../../lib/timezone";
import type { ActiveFilterChip } from "./ActiveFilters";
import {
  calendarGrid,
  calendarMonth,
  calendarMonthLabel,
  calendarMonths,
  calendarNavNext,
  calendarNavPrev,
  dateContent,
  dateContentLayout,
  dateInput,
  datePickerControl,
  datePresetButton,
  datePresetList,
  dateRangeCluster,
  dateRangeDayCount,
  dateRangeDivider,
  dateRangeShell,
  dateRangeTimezone,
  filtersBar,
  filtersBarAccount,
  filtersBarBrandLink,
  filtersBarLogoImage,
  periodNavButtonNext,
  periodNavButtonPrev,
  periodNavIcon,
  segmentFiltersCluster,
} from "./DashboardFilters.css";
import DownloadDbButton from "./DownloadDbButton";
import { useDashboardState } from "./dashboardPeriodContext";
import FilterChip from "./FilterChip";

// type DashboardSurface = "product" | "marketing";

const MONTH_OFFSETS = [0, 1];
const WEEKDAY_LETTERS = ["S", "M", "T", "W", "T", "F", "S"];

function isDateValue(value: unknown): value is DateValueLike {
  return (
    typeof value === "object" &&
    value !== null &&
    "year" in value &&
    "month" in value &&
    "day" in value
  );
}

function rangeFromDateValues(timeZone: string, values: unknown[]): DateRangeSeconds | null {
  const startValue = values[0];
  const endValue = values[values.length - 1] ?? values[0];
  if (!isDateValue(startValue) || !isDateValue(endValue)) {
    return null;
  }
  return {
    start: dateValueToUnixStart(timeZone, startValue),
    end: dateValueToUnixEndExclusive(timeZone, endValue),
  };
}

function dateValuesFromPickerValue(values: unknown[]): DateValueLike[] | null {
  const dateValues: DateValueLike[] = [];
  for (const value of values) {
    if (!isDateValue(value)) {
      return null;
    }
    dateValues.push(value);
  }
  return dateValues;
}

function dateValueToIso(value: DateValueLike): string {
  const month = String(value.month).padStart(2, "0");
  const day = String(value.day).padStart(2, "0");
  return `${value.year}-${month}-${day}`;
}

function rangeToPickerValue(timeZone: string, range: DateRangeSeconds): DateValueLike[] {
  return [
    unixStartToDateValue(timeZone, range.start),
    unixStartToDateValue(timeZone, Math.max(range.start, range.end - 1)),
  ];
}

function todayDateValue(timeZone: string): DateValueLike {
  return unixStartToDateValue(timeZone, nowUnixSeconds());
}

// Picker values are civil dates, so ordering is calendar ordering and needs no
// zone: only turning one into an instant does.
function compareDateValues(left: DateValueLike, right: DateValueLike): number {
  return left.year - right.year || left.month - right.month || left.day - right.day;
}

function isBeforeDate(left: DateValueLike, right: DateValueLike): boolean {
  return compareDateValues(left, right) < 0;
}

function isSameDate(left: DateValueLike, right: DateValueLike): boolean {
  return compareDateValues(left, right) === 0;
}

function isSameCalendarMonth(left: DateValueLike, right: DateValueLike): boolean {
  return left.year === right.year && left.month === right.month;
}

function formatMonthLabel(value: DateValueLike): string {
  // A civil date read as UTC, purely to get a month name out of it.
  const date = new Date(Date.UTC(value.year, value.month - 1, value.day));
  // Fixed locale so SSR/client never disagree on month names.
  return date.toLocaleDateString("en-US", {
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
}

function formatDayCountLabel(count: number): string {
  return count === 1 ? "1 day selected" : `${count} days selected`;
}

type DashboardFiltersProps = {
  showBrand?: boolean;
  /** Account menu; off for the marketing embed where the site header already has it. */
  showAccount?: boolean;
  projects?: ProjectOption[];
  projectId?: string;
  onProjectChange?: (projectId: string) => void;
  /** When true, date range controls are inactive (e.g. Revenue tab uses a fixed window). */
  dateRangeDisabled?: boolean;
};

export default function DashboardFilters(props: DashboardFiltersProps) {
  const showBrand = () => props.showBrand ?? true;
  const showAccount = () => props.showAccount ?? true;
  const dateRangeDisabled = () => props.dateRangeDisabled ?? false;
  // Product vs marketing surface; enable with the SegmentedControl below when we start product analytics views.
  // const [surface, setSurface] = createSignal<DashboardSurface>("marketing");
  const {
    timeZone,
    range,
    setRange,
    rangeReady,
    segments,
    removeCountryFilter,
    removeSourceFilter,
    removePageFilter,
    removeLandingPageFilter,
    removeEventFilter,
    removeAttributeFilter,
  } = useDashboardState();
  const segmentChips = createMemo((): ActiveFilterChip[] => {
    const current = segments();
    return [
      ...current.countries.map((country) => ({
        label: `Country: ${country.name}`,
        onRemove: () => removeCountryFilter(country.code),
      })),
      ...current.sources.map((source) => ({
        label: `Source: ${source.name}`,
        onRemove: () => removeSourceFilter(sourceFilterKey(source)),
      })),
      ...current.pages.map((page) => ({
        label: `Page: ${page.name}`,
        onRemove: () => removePageFilter(pageFilterKey(page)),
      })),
      ...current.landingPages.map((page) => ({
        label: `Landing page: ${page.name}`,
        onRemove: () => removeLandingPageFilter(landingPageFilterKey(page)),
      })),
      ...current.events.map((event) => ({
        label: `Event: ${event.name}`,
        onRemove: () => removeEventFilter(eventFilterKey(event)),
      })),
      ...current.attributes.map((filter) => ({
        label: `${LANDING_ATTRIBUTE_LABELS[filter.attribute]}: ${filter.name}`,
        onRemove: () => removeAttributeFilter(attributeFilterKey(filter)),
      })),
    ];
  });
  const [shortcut, setShortcut] = createSignal<DateShortcut | "custom">(DEFAULT_DATE_SHORTCUT);
  // Empty until rangeReady so SSR and the first client paint match (placeholder inputs).
  const [pickerValue, setPickerValue] = createSignal<unknown[]>([]);
  const [pendingStartValue, setPendingStartValue] = createSignal<DateValueLike | null>(null);
  const [calendarOpen, setCalendarOpen] = createSignal(false);
  // Calendar grid is today-relative; keep it out of SSR HTML.
  const [calendarMounted, setCalendarMounted] = createSignal(false);
  let suppressPickerValueChange = false;

  onMount(() => {
    setCalendarMounted(true);
  });

  // Clock-dependent, so it stays null until mount rather than SSRing a date, and
  // re-derives once the project's zone arrives with the account.
  const today = createMemo(() => (calendarMounted() ? todayDateValue(timeZone()) : null));
  const timezoneLabel = createMemo(() => formatTimeZoneLabel(timeZone()));

  createEffect(() => {
    if (!rangeReady() || pendingStartValue()) {
      return;
    }
    setPickerValue(rangeToPickerValue(timeZone(), range()));
  });

  // The provider may open on a range carried over from another project rather
  // than the default preset; label whichever it is, once.
  let shortcutLabelled = false;
  createEffect(() => {
    if (shortcutLabelled || !rangeReady()) {
      return;
    }
    shortcutLabelled = true;
    setShortcut(shortcutForRange(untrack(range), untrack(timeZone)));
  });

  createEffect(() => {
    if (dateRangeDisabled()) {
      setCalendarOpen(false);
      setPendingStartValue(null);
    }
  });

  const pickerDateValues = createMemo(() =>
    pickerValue().map((entry) => {
      if (!isDateValue(entry)) {
        return parseDate("2024-01-01");
      }
      return parseDate(dateValueToIso(entry));
    }),
  );

  const maxDate = createMemo(() => {
    const value = today();
    return value ? parseDate(dateValueToIso(value)) : undefined;
  });

  const canShiftNext = createMemo(() => {
    const todayValue = today();
    if (!rangeReady() || !todayValue) {
      return false;
    }
    const endDate = rangeToPickerValue(timeZone(), range())[1];
    return isBeforeDate(endDate, todayValue);
  });

  const dayCountLabel = createMemo(() => {
    const pending = pendingStartValue();
    if (pending && pickerValue().length < 2) {
      return "Select end date";
    }
    const fromPicker = rangeFromDateValues(timeZone(), pickerValue());
    if (fromPicker) {
      return formatDayCountLabel(rangeDayCount(fromPicker));
    }
    if (!rangeReady()) {
      return "";
    }
    return formatDayCountLabel(rangeDayCount(range()));
  });

  const selectShortcut = (next: DateShortcut) => {
    setShortcut(next);
    const nextRange = shortcutToRange(next, timeZone());
    setCalendarOpen(false);
    setPendingStartValue(null);
    setRange(nextRange);
    setPickerValue(rangeToPickerValue(timeZone(), nextRange));
  };

  const setPendingStart = (startValue: DateValueLike) => {
    setPendingStartValue(startValue);
    setPickerValue([startValue]);
  };

  const commitPickerRange = (dateValues: DateValueLike[]) => {
    const nextRange = rangeFromDateValues(timeZone(), dateValues);
    if (!nextRange) {
      setPickerValue(dateValues);
      return;
    }
    const clampedRange = clampRangeToMaxDays(nextRange);
    setPendingStartValue(null);
    setCalendarOpen(false);
    setRange(clampedRange);
    setPickerValue(rangeToPickerValue(timeZone(), clampedRange));
  };

  const suppressInternalPickerValueChange = () => {
    suppressPickerValueChange = true;
    queueMicrotask(() => {
      suppressPickerValueChange = false;
    });
  };

  const selectCalendarDay = (day: DateValueLike, visibleMonthStart: DateValueLike) => {
    const todayValue = today();
    if (
      !todayValue ||
      !isSameCalendarMonth(day, visibleMonthStart) ||
      isBeforeDate(todayValue, day)
    ) {
      return;
    }
    setShortcut("custom");
    const pendingStart = pendingStartValue();
    if (!pendingStart || isBeforeDate(day, pendingStart)) {
      setPendingStart(day);
      return;
    }
    commitPickerRange([pendingStart, day]);
  };

  const onPickerChange = (details: { value: unknown[] }) => {
    if (suppressPickerValueChange) {
      return;
    }
    setShortcut("custom");
    const dateValues = dateValuesFromPickerValue(details.value);
    if (!dateValues || dateValues.length === 0) {
      setPendingStartValue(null);
      setPickerValue(details.value);
      return;
    }
    if (dateValues.length === 1) {
      setPendingStart(dateValues[0]);
      return;
    }
    const pendingStart = pendingStartValue();
    if (
      pendingStart &&
      isSameDate(dateValues[1], pendingStart) &&
      isBeforeDate(dateValues[0], pendingStart)
    ) {
      setPendingStart(dateValues[0]);
      return;
    }
    commitPickerRange(dateValues);
  };

  const shiftRange = (direction: -1 | 1) => {
    if (!rangeReady() || (direction === 1 && !canShiftNext())) {
      return;
    }
    const todayValue = today();
    if (!todayValue) {
      return;
    }
    const nextRange = shiftDateRange(timeZone(), range(), direction);
    if (direction === 1) {
      const todayEndExclusive = dateValueToUnixEndExclusive(timeZone(), todayValue);
      if (nextRange.end > todayEndExclusive) {
        nextRange.end = todayEndExclusive;
      }
    }
    setShortcut("custom");
    setPendingStartValue(null);
    setRange(nextRange);
    setPickerValue(rangeToPickerValue(timeZone(), nextRange));
  };

  return (
    <div class={filtersBar}>
      <Show when={showBrand()}>
        <a href="/" class={filtersBarBrandLink} aria-label="RevScope home">
          <img
            class={filtersBarLogoImage}
            src={revscopeLogo.src}
            width={revscopeLogo.width}
            height={revscopeLogo.height}
            alt=""
          />
        </a>
      </Show>
      {/* Enable when we start developing the product analytics views.
      <SegmentedControl
        value={surface()}
        ariaLabel="Dashboard surface"
        options={[
          { value: "product", label: "Product" },
          { value: "marketing", label: "Marketing" },
        ]}
        onChange={setSurface}
      />
      */}
      <div
        class={dateRangeCluster}
        data-disabled={dateRangeDisabled() ? "" : undefined}
        title={
          dateRangeDisabled() ? "Revenue shows the last 12 months of payment activity" : undefined
        }
      >
        <button
          type="button"
          class={periodNavButtonPrev}
          aria-label="Previous period"
          disabled={!rangeReady() || dateRangeDisabled()}
          onClick={() => shiftRange(-1)}
        >
          <span class={periodNavIcon} aria-hidden="true">
            ‹
          </span>
        </button>
        <div class={dateRangeShell}>
          <DatePicker.Root
            selectionMode="range"
            numOfMonths={2}
            openOnClick
            disabled={dateRangeDisabled()}
            open={calendarOpen()}
            onOpenChange={(details) => {
              if (!rangeReady() || dateRangeDisabled()) {
                setCalendarOpen(false);
                return;
              }
              setCalendarOpen(details.open);
            }}
            max={maxDate()}
            value={pickerDateValues()}
            onValueChange={onPickerChange}
          >
            <DatePicker.Control class={datePickerControl}>
              <DatePicker.Input
                index={0}
                class={dateInput}
                readOnly={!rangeReady() || dateRangeDisabled()}
              />
              <span class={dateRangeDivider} aria-hidden="true">
                →
              </span>
              <DatePicker.Input
                index={1}
                class={dateInput}
                readOnly={!rangeReady() || dateRangeDisabled()}
              />
            </DatePicker.Control>
            <Show when={calendarMounted()}>
              <DatePicker.Positioner>
                <DatePicker.Content class={dateContent}>
                  <div class={dateContentLayout}>
                    <div class={datePresetList} role="listbox" aria-label="Date range presets">
                      <For each={DATE_SHORTCUT_OPTIONS}>
                        {(option) => (
                          <button
                            type="button"
                            role="option"
                            class={datePresetButton}
                            aria-selected={shortcut() === option.value}
                            data-selected={shortcut() === option.value ? "" : undefined}
                            onClick={() => selectShortcut(option.value)}
                          >
                            {option.label}
                          </button>
                        )}
                      </For>
                    </div>
                    <DatePicker.View view="day">
                      <DatePicker.Context>
                        {(api) => (
                          <div class={calendarMonths}>
                            <DatePicker.PrevTrigger class={calendarNavPrev}>
                              ‹
                            </DatePicker.PrevTrigger>
                            <DatePicker.NextTrigger class={calendarNavNext}>
                              ›
                            </DatePicker.NextTrigger>
                            <For each={MONTH_OFFSETS}>
                              {(monthOffset) => {
                                const visibleRange = () =>
                                  monthOffset === 0
                                    ? api().visibleRange
                                    : api().getOffset({ months: monthOffset }).visibleRange;
                                const weeks = () => api().getMonthWeeks(visibleRange().start);

                                return (
                                  <div class={calendarMonth}>
                                    <div class={calendarMonthLabel}>
                                      {formatMonthLabel(visibleRange().start)}
                                    </div>
                                    <DatePicker.Table class={calendarGrid}>
                                      <DatePicker.TableHead>
                                        <DatePicker.TableRow>
                                          <Index each={WEEKDAY_LETTERS}>
                                            {(weekday) => (
                                              <DatePicker.TableHeader>
                                                {weekday()}
                                              </DatePicker.TableHeader>
                                            )}
                                          </Index>
                                        </DatePicker.TableRow>
                                      </DatePicker.TableHead>
                                      <DatePicker.TableBody>
                                        <For each={weeks()}>
                                          {(week) => (
                                            <DatePicker.TableRow>
                                              <For each={week}>
                                                {(day) => (
                                                  <DatePicker.TableCell
                                                    value={day}
                                                    visibleRange={visibleRange()}
                                                  >
                                                    <DatePicker.TableCellTrigger
                                                      onClick={(event) => {
                                                        event.preventDefault();
                                                        event.stopPropagation();
                                                        event.stopImmediatePropagation();
                                                        suppressInternalPickerValueChange();
                                                        selectCalendarDay(
                                                          day,
                                                          visibleRange().start,
                                                        );
                                                      }}
                                                    >
                                                      {day.day}
                                                    </DatePicker.TableCellTrigger>
                                                  </DatePicker.TableCell>
                                                )}
                                              </For>
                                            </DatePicker.TableRow>
                                          )}
                                        </For>
                                      </DatePicker.TableBody>
                                    </DatePicker.Table>
                                  </div>
                                );
                              }}
                            </For>
                          </div>
                        )}
                      </DatePicker.Context>
                    </DatePicker.View>
                  </div>
                  <Show when={dayCountLabel()}>
                    {(label) => (
                      <div class={dateRangeDayCount} aria-live="polite">
                        {label()}
                      </div>
                    )}
                  </Show>
                </DatePicker.Content>
              </DatePicker.Positioner>
            </Show>
          </DatePicker.Root>
        </div>
        <button
          type="button"
          class={periodNavButtonNext}
          aria-label="Next period"
          disabled={!canShiftNext() || dateRangeDisabled()}
          onClick={() => shiftRange(1)}
        >
          <span class={periodNavIcon} aria-hidden="true">
            ›
          </span>
        </button>
      </div>
      <a
        class={dateRangeTimezone}
        href={appPath("/settings#timezone")}
        title="Change project timezone"
      >
        {timezoneLabel()}
      </a>
      <Show when={hasSegmentFilters(segments())}>
        <div class={segmentFiltersCluster}>
          <For each={segmentChips()}>{(chip) => <FilterChip {...chip} />}</For>
        </div>
      </Show>
      <Show when={showAccount()}>
        <div class={filtersBarAccount}>
          <LoggedInUser
            projects={props.projects}
            projectId={props.projectId}
            onProjectChange={(projectId) => props.onProjectChange?.(projectId)}
          />
          <DownloadDbButton projectId={props.projectId} />
        </div>
      </Show>
    </div>
  );
}
