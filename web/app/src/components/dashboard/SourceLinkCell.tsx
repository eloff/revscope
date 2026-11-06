/// <reference types="solid-js" />
import { Show } from "solid-js";
import type { PaymentLanding } from "../../lib/paymentsApi";
import {
  type AttributedLanding,
  landingRevenueSourceTarget,
  landingSourceDetail,
  landingSourceGroup,
  landingSourceSubtitle,
} from "./conversionsLogic";
import { gridGroupBadge } from "./grid.css";
import {
  type RevenueSourceLinkTarget,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  type SourceGroup,
} from "./revenueSourcesLogic";
import ViewDetailsButton from "./ViewDetailsButton";

type SourceLinkCellProps = {
  landing: AttributedLanding | PaymentLanding | undefined;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
};

function groupBadgeStyle(group: SourceGroup): {
  background: string;
  color: string;
} {
  return {
    background: SOURCE_GROUP_COLORS[group],
    color: "#ffffff",
  };
}

export default function SourceLinkCell(props: SourceLinkCellProps) {
  const group = () => landingSourceGroup(props.landing);
  const detail = () => landingSourceDetail(props.landing);
  const subtitle = () => landingSourceSubtitle(props.landing);
  const target = () => landingRevenueSourceTarget(props.landing);

  return (
    <Show when={group()} fallback="—">
      {(sourceGroup) => (
        <>
          <span class={gridGroupBadge} style={groupBadgeStyle(sourceGroup())}>
            {SOURCE_GROUP_LABELS[sourceGroup()]}
          </span>{" "}
          <Show when={props.onRevenueSourceSelect && target()} fallback={detail()}>
            {(linkTarget) => (
              <ViewDetailsButton
                title={`View details for ${subtitle()} in Sources`}
                onClick={(event) => {
                  event.stopPropagation();
                  props.onRevenueSourceSelect?.(linkTarget());
                }}
              >
                {subtitle()}
              </ViewDetailsButton>
            )}
          </Show>
        </>
      )}
    </Show>
  );
}
