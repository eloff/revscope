import { Show } from "solid-js";
import {
  funnelStepVisitors,
  funnelStepVisitorsCount,
  funnelStepVisitorsMuted,
} from "./FunnelStepVisitorCount.css";

/** The editor preview's visitor count for one step. */
export default function FunnelStepVisitorCount(props: {
  visitors: number | undefined;
  loading: boolean;
}) {
  return (
    <span class={funnelStepVisitors}>
      <Show
        when={props.visitors !== undefined}
        fallback={
          <span class={funnelStepVisitorsMuted}>
            {props.loading ? "Calculating visitors..." : "No visitor data"}
          </span>
        }
      >
        <span class={funnelStepVisitorsCount}>{(props.visitors ?? 0).toLocaleString()}</span>
        <span>visitors</span>
      </Show>
    </span>
  );
}
