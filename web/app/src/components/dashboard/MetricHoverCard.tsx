/// <reference types="solid-js" />

import type { PolymorphicProps } from "@ark-ui/solid/factory";
import { HoverCard } from "@ark-ui/solid/hover-card";
import type { JSX } from "solid-js";
import { Portal } from "solid-js/web";
import {
  hoverCardContent,
  hoverCardPositioner,
  overviewMetricHoverCardContent,
  wideHoverCardContent,
} from "./hoverCard.css";
import { hoverCardPortalMount } from "./hoverCardPortal";
import MetricHoverCardContent from "./MetricHoverCardContent";
import type { MetricHoverContent } from "./metricHoverDetails";

export type MetricHoverCardSize = "default" | "overview" | "revenueSource";

export type MetricHoverCardProps = {
  title: string;
  content: MetricHoverContent | undefined;
  loading?: boolean;
  statLayout?: "twoColumn" | "threeColumn";
  size?: MetricHoverCardSize;
  onLandingPathSelect?: (pathId: number) => void;
  showPeriodComparisonNote?: boolean;
  renderTrigger: (mergeProps: <T extends object>(props: T) => T) => JSX.Element;
};

export default function MetricHoverCard(props: MetricHoverCardProps) {
  const contentClass = () => {
    switch (props.size) {
      case "overview":
        return overviewMetricHoverCardContent;
      case "revenueSource":
        return wideHoverCardContent;
      default:
        return hoverCardContent;
    }
  };

  return (
    <HoverCard.Root
      openDelay={300}
      closeDelay={150}
      positioning={{ placement: "top", strategy: "fixed" }}
    >
      <HoverCard.Trigger
        asChild={props.renderTrigger as NonNullable<PolymorphicProps<"button">["asChild"]>}
      />
      <Portal mount={hoverCardPortalMount()}>
        <HoverCard.Positioner class={hoverCardPositioner}>
          <HoverCard.Content class={contentClass()}>
            <MetricHoverCardContent
              title={props.title}
              content={props.content}
              loading={props.loading}
              statLayout={props.statLayout}
              onLandingPathSelect={props.onLandingPathSelect}
              showPeriodComparisonNote={props.showPeriodComparisonNote}
            />
          </HoverCard.Content>
        </HoverCard.Positioner>
      </Portal>
    </HoverCard.Root>
  );
}
