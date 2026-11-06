import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { FunnelDataResponse } from "../../lib/funnelsApi";
import ApexCharts from "./apexcharts";
import {
  funnelChartCard,
  funnelChartMount,
  funnelChartStack,
  funnelDropoffHeading,
  funnelStepDetailColumn,
  funnelStepDetailsGrid,
  funnelStepDetailsPanel,
  funnelStepDetailsPlaceholder,
} from "./FunnelChart.css";
import FunnelDropoffTreemap from "./FunnelDropoffTreemap";
import {
  buildChartOptions,
  buildStepDetailContent,
  highestDropoffStepOrder,
  sortFunnelSteps,
  stepDisplayName,
} from "./funnelChartOptions";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { errorMessage, message } from "./messages.css";

/** Delay before clearing hover so leave/enter between adjacent steps does not flash. */
const FUNNEL_HOVER_CLEAR_DELAY_MS = 120;

function scrollPanelFullyIntoView(element: HTMLElement) {
  const rect = element.getBoundingClientRect();
  const margin = 16;
  const viewportHeight = window.innerHeight;
  const fitsInViewport = rect.height <= viewportHeight - 2 * margin;
  const fullyVisible = rect.top >= margin && rect.bottom <= viewportHeight - margin;

  if (fullyVisible) {
    return;
  }

  const block = fitsInViewport && rect.bottom > viewportHeight - margin ? "end" : "start";
  element.scrollIntoView({ behavior: "smooth", block });
}

export default function FunnelChart(props: {
  data?: FunnelDataResponse;
  loading: boolean;
  error: string;
}) {
  let chartElement: HTMLDivElement | undefined;
  let stepDetailsPanelElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  let hoverClearTimer: ReturnType<typeof setTimeout> | undefined;
  // Click pins a step; hover can preview other steps, then falls back to the pin.
  const [pinnedIndex, setPinnedIndex] = createSignal<number | null>(null);
  const [hoveredIndex, setHoveredIndex] = createSignal<number | null>(null);

  const sortedSteps = createMemo(() => (props.data ? sortFunnelSteps(props.data.steps) : []));
  const hasChartSteps = createMemo(
    () => !props.loading && !props.error && (props.data?.steps.length ?? 0) > 0,
  );
  const activeIndex = createMemo(() => hoveredIndex() ?? pinnedIndex());
  const activeStep = createMemo(() => {
    const index = activeIndex();
    return index === null ? undefined : sortedSteps()[index];
  });
  const highestDropoffOrder = createMemo(() => highestDropoffStepOrder(sortedSteps()));

  const clearHoverClearTimer = () => {
    if (hoverClearTimer !== undefined) {
      clearTimeout(hoverClearTimer);
      hoverClearTimer = undefined;
    }
  };

  // Enter updates immediately; leave is delayed so moving between adjacent steps
  // does not blank the panel between Apex leave/enter events.
  const setHoveredStep = (index: number | null) => {
    clearHoverClearTimer();
    if (index !== null) {
      setHoveredIndex(index);
      return;
    }
    hoverClearTimer = setTimeout(() => {
      hoverClearTimer = undefined;
      setHoveredIndex(null);
    }, FUNNEL_HOVER_CLEAR_DELAY_MS);
  };

  // Only scroll when the user pins a step — never on hover preview.
  createEffect(() => {
    const index = pinnedIndex();
    if (index === null) {
      return;
    }

    let cancelled = false;
    const frame = requestAnimationFrame(() => {
      if (cancelled || !stepDetailsPanelElement) {
        return;
      }
      scrollPanelFullyIntoView(stepDetailsPanelElement);
    });
    onCleanup(() => {
      cancelled = true;
      cancelAnimationFrame(frame);
    });
  });

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "bar", height: 540 },
      series: [],
    });
    chart.render();
  });

  createEffect(() => {
    props.data;
    clearHoverClearTimer();
    setPinnedIndex(null);
    setHoveredIndex(null);
  });

  createEffect(() => {
    if (!chart || props.loading || props.error) {
      return;
    }
    const data = props.data;
    if (!data) {
      return;
    }
    chart.updateOptions(
      buildChartOptions(data, {
        onStepClick: (index) => {
          setPinnedIndex((current) => (current === index ? null : index));
        },
        onStepHover: setHoveredStep,
      }),
      true,
      true,
    );
  });

  onCleanup(() => {
    clearHoverClearTimer();
    chart?.destroy();
    chart = undefined;
  });

  return (
    <div class={funnelChartStack}>
      <div class={funnelChartCard}>
        <Show when={props.loading}>
          <p class={message}>Loading funnel data...</p>
        </Show>
        <Show when={!props.loading && props.error}>
          <p class={errorMessage}>{props.error}</p>
        </Show>
        <Show when={!props.loading && !props.error && props.data?.steps.length === 0}>
          <p class={message}>Add steps to this funnel to chart visitor counts.</p>
        </Show>
        <div
          ref={chartElement}
          class={funnelChartMount}
          style={{
            display:
              props.loading || props.error || props.data?.steps.length === 0 ? "none" : "block",
          }}
        />
      </div>
      <Show when={hasChartSteps()}>
        <div class={funnelStepDetailsPanel} ref={stepDetailsPanelElement}>
          <Show
            when={activeStep()}
            fallback={
              <div class={funnelStepDetailsPlaceholder}>
                Hover or click a step to see details and drop-off destinations.
              </div>
            }
          >
            {(step) => (
              <div class={funnelStepDetailsGrid}>
                <div class={funnelStepDetailColumn}>
                  <MetricHoverCardContent
                    title={stepDisplayName(step())}
                    content={buildStepDetailContent(
                      step(),
                      props.data?.total_revenue,
                      step().order === highestDropoffOrder(),
                    )}
                    statLayout="threeColumn"
                  />
                </div>
                <div class={funnelStepDetailColumn}>
                  <div class={funnelDropoffHeading}>Drop-off destinations</div>
                  <Show
                    when={step().dropoff}
                    fallback={
                      <p class={message}>No next step drop-off destinations for this step.</p>
                    }
                  >
                    {(dropoff) => (
                      <Show
                        when={dropoff().transitions.length > 0}
                        fallback={<p class={message}>No next-page transitions for this step.</p>}
                      >
                        <FunnelDropoffTreemap transitions={dropoff().transitions} />
                      </Show>
                    )}
                  </Show>
                </div>
              </div>
            )}
          </Show>
        </div>
      </Show>
    </div>
  );
}
