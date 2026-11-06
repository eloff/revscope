/// <reference types="solid-js" />
import { For, type JSX, Show } from "solid-js";
import { formatPercentRatio } from "../../lib/dashboardApi";
import { type ChangeTone, changeToneForRatio, metricChangeClass } from "./changeTone";
import HoverHighlightDescriptions from "./HoverHighlightDescriptions";
import {
  hoverCardDescription,
  hoverCardStatLabel,
  hoverCardStatRow,
  hoverCardStats,
  hoverCardStatValue,
  hoverCardTitle,
} from "./hoverCard.css";
import {
  hoverCardLandingPathsHeaderRow,
  hoverCardLandingPathsSection,
  hoverCardLandingPathsVisitorsHeader,
  hoverCardStatChange,
  hoverCardStatRowThreeCol,
  hoverCardStatSectionHeader,
  hoverCardStatValueNegative,
  hoverCardStatValueNeutral,
  hoverCardStatValuePositive,
} from "./MetricHoverCardContent.css";
import type { HoverStatLine, MetricHoverContent } from "./metricHoverDetails";
import PeriodComparisonNote from "./PeriodComparisonNote";
import { FLAT_CHANGE_TEXT, formatTileChangeRatio } from "./revenueSourcesLogic";
import ViewDetailsButton from "./ViewDetailsButton";

export type MetricHoverCardContentProps = {
  title: string;
  content: MetricHoverContent | undefined;
  loading?: boolean;
  statLayout?: "twoColumn" | "threeColumn";
  onLandingPathSelect?: (pathId: number) => void;
  showPeriodComparisonNote?: boolean;
};

const VALUE_COLUMN_CHANGE_CLASS: Record<ChangeTone, string> = {
  positive: hoverCardStatValuePositive,
  negative: hoverCardStatValueNegative,
  neutral: hoverCardStatValueNeutral,
};

function renderChangeRatioBadge(line: HoverStatLine): JSX.Element | null {
  const ratio = line.changeRatio;
  if (ratio == null || ratio === 0) {
    return null;
  }
  const changeText = formatTileChangeRatio(ratio);
  if (!changeText) {
    return null;
  }
  const tone = changeToneForRatio(ratio, line.higherIsBetter ?? true);
  const toneClass = line.changeInValueColumn
    ? VALUE_COLUMN_CHANGE_CLASS[tone]
    : metricChangeClass(tone);
  return <span class={toneClass}>{changeText}</span>;
}

function renderStatChange(line: HoverStatLine): JSX.Element | null {
  return renderChangeRatioBadge(line);
}

function renderStatValue(line: HoverStatLine): JSX.Element {
  const badge = renderChangeRatioBadge(line);
  if (line.changeInValueColumn) {
    const ratio = line.changeRatio;
    // A flat change is still a value here, unlike a missing comparison.
    if (badge || ratio == null || !Number.isFinite(ratio)) {
      return badge ?? "—";
    }
    return <span class={VALUE_COLUMN_CHANGE_CLASS.neutral}>{FLAT_CHANGE_TEXT}</span>;
  }
  return badge ? (
    <>
      {line.value} {badge}
    </>
  ) : (
    line.value
  );
}

function renderStatRow(
  line: HoverStatLine,
  layout: "twoColumn" | "threeColumn",
  statChangeClass: string,
): JSX.Element {
  if (layout === "threeColumn") {
    return (
      <div class={hoverCardStatRowThreeCol}>
        <dt class={hoverCardStatLabel}>{line.label}</dt>
        <dd class={hoverCardStatValue}>
          {line.changeInValueColumn ? renderStatValue(line) : line.value}
        </dd>
        <dd class={statChangeClass}>{line.changeInValueColumn ? null : renderStatChange(line)}</dd>
      </div>
    );
  }
  return (
    <div class={hoverCardStatRow}>
      <dt class={hoverCardStatLabel}>{line.label}</dt>
      <dd class={hoverCardStatValue}>{renderStatValue(line)}</dd>
    </div>
  );
}

function renderLandingPathAction(
  entry: NonNullable<MetricHoverContent["landingPaths"]>[number],
  onLandingPathSelect: MetricHoverCardContentProps["onLandingPathSelect"],
): JSX.Element {
  if (!onLandingPathSelect) {
    return <>{entry.path}</>;
  }
  return (
    <ViewDetailsButton
      title={`View details for ${entry.path} in Landing Pages`}
      onClick={(event) => {
        event.stopPropagation();
        onLandingPathSelect(entry.id);
      }}
    >
      {entry.path}
    </ViewDetailsButton>
  );
}

function formatLandingPathCount(
  entry: NonNullable<MetricHoverContent["landingPaths"]>[number],
): string {
  const count = entry.visitors.toLocaleString();
  if (entry.share == null) {
    return count;
  }
  return `${count} (${formatPercentRatio(entry.share, 1)})`;
}

function renderLandingPathShare(
  entry: NonNullable<MetricHoverContent["landingPaths"]>[number],
): JSX.Element | null {
  if (entry.share == null) {
    return null;
  }
  return formatPercentRatio(entry.share, 1);
}

function renderLandingPathsSection(
  landingPaths: MetricHoverContent["landingPaths"],
  layout: "twoColumn" | "threeColumn",
  statChangeClass: string,
  onLandingPathSelect: MetricHoverCardContentProps["onLandingPathSelect"],
  title: string,
  countLabel: string,
  shareLabel: string,
): JSX.Element {
  const paths = landingPaths ?? [];
  if (paths.length === 0) {
    return null;
  }
  const showShare = paths.some((entry) => entry.share != null);

  return (
    <div class={hoverCardLandingPathsSection}>
      {layout === "threeColumn" ? (
        <div class={`${hoverCardStatRowThreeCol} ${hoverCardLandingPathsHeaderRow}`}>
          <dt class={hoverCardStatSectionHeader}>{title}</dt>
          <dd class={`${hoverCardStatSectionHeader} ${hoverCardLandingPathsVisitorsHeader}`}>
            {countLabel}
          </dd>
          <dd class={`${hoverCardStatSectionHeader} ${statChangeClass}`}>
            {showShare ? shareLabel : null}
          </dd>
        </div>
      ) : (
        <div class={`${hoverCardStatRow} ${hoverCardLandingPathsHeaderRow}`}>
          <div class={hoverCardStatSectionHeader}>{title}</div>
          <div class={hoverCardStatSectionHeader}>{countLabel}</div>
        </div>
      )}
      <For each={paths}>
        {(entry) =>
          layout === "threeColumn" ? (
            <div class={hoverCardStatRowThreeCol}>
              <dt class={hoverCardStatLabel}>
                {renderLandingPathAction(entry, onLandingPathSelect)}
              </dt>
              <dd class={hoverCardStatValue}>{entry.visitors.toLocaleString()}</dd>
              <dd class={statChangeClass}>{renderLandingPathShare(entry)}</dd>
            </div>
          ) : (
            <div class={hoverCardStatRow}>
              <dt class={hoverCardStatLabel}>
                {renderLandingPathAction(entry, onLandingPathSelect)}
              </dt>
              <dd class={hoverCardStatValue}>{formatLandingPathCount(entry)}</dd>
            </div>
          )
        }
      </For>
    </div>
  );
}

export default function MetricHoverCardContent(props: MetricHoverCardContentProps) {
  const statLayout = () => props.statLayout ?? "twoColumn";

  return (
    <>
      <div class={hoverCardTitle}>{props.title}</div>
      <Show
        when={!props.loading && props.content}
        fallback={
          <p class={hoverCardDescription}>
            {props.loading ? "Loading…" : "No data for this period."}
          </p>
        }
      >
        {(detail) => (
          <>
            <dl class={hoverCardStats}>
              <For each={detail().stats}>
                {(line) => renderStatRow(line, statLayout(), hoverCardStatChange)}
              </For>
              {renderLandingPathsSection(
                detail().landingPaths,
                statLayout(),
                hoverCardStatChange,
                props.onLandingPathSelect,
                detail().landingPathsTitle ?? "Top landing paths",
                detail().landingPathsCountLabel ?? "Visitors",
                detail().landingPathsShareLabel ?? "%",
              )}
              {renderLandingPathsSection(
                detail().referrers,
                statLayout(),
                hoverCardStatChange,
                undefined,
                detail().referrersTitle ?? "Top referrers",
                "Visitors",
                "%",
              )}
            </dl>
            <Show when={detail().highlightDescriptions}>
              {(lines) => <HoverHighlightDescriptions lines={lines()} />}
            </Show>
            <Show when={detail().description}>
              {(description) => <p class={hoverCardDescription}>{description()}</p>}
            </Show>
            <Show when={detail().footnote}>
              {(footnote) => <p class={hoverCardDescription}>{footnote()}</p>}
            </Show>
            <Show when={props.showPeriodComparisonNote ?? true}>
              <PeriodComparisonNote variant="inline" descriptionClass={hoverCardDescription} />
            </Show>
          </>
        )}
      </Show>
    </>
  );
}
