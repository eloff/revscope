/// <reference types="solid-js" />
import { Show } from "solid-js";
import type { MetricsResponse, OverviewResponse } from "../../lib/dashboardApi";
import { hasSegmentFilters } from "../../lib/segmentFilters";
import { useDashboardState } from "./dashboardPeriodContext";
import { useImportedCoverage } from "./ImportedCoverageContext";
import MetricsBar from "./MetricsBar";
import NoDataView from "./NoDataView";
import OverviewBreakdowns from "./OverviewBreakdowns";
import OverviewChart from "./OverviewChart";
import { chartPanel } from "./OverviewTabContent.css";

type OverviewTabContentProps = {
  data: OverviewResponse | undefined;
  metrics: MetricsResponse | undefined;
  loading: boolean;
  error: string;
  empty: boolean;
};

export default function OverviewTabContent(props: OverviewTabContentProps) {
  const coverage = useImportedCoverage();
  const { segments } = useDashboardState();
  // A chart of tracked data alone has nothing imported on it to point out, and
  // that's what the setting and any filter leave.
  const importedRanges = () => {
    const current = coverage();
    return current?.include_in_reports && !hasSegmentFilters(segments()) ? current.ranges : [];
  };
  return (
    <Show when={!props.empty} fallback={<NoDataView />}>
      <MetricsBar
        data={props.metrics}
        overview={props.data}
        loading={props.loading && !props.data}
      />
      {/* The home page embed sizes its collapsed frame to end here. */}
      <div class={chartPanel} data-embed-fold>
        <OverviewChart
          data={props.data}
          loading={props.loading && !props.data}
          error={props.error}
          importedRanges={importedRanges()}
        />
      </div>
      <OverviewBreakdowns data={props.data?.breakdowns} loading={props.loading} />
    </Show>
  );
}
