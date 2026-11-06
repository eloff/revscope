/// <reference types="solid-js" />

import { useDashboardState } from "./dashboardPeriodContext";
import { hoverCardDescription } from "./hoverCard.css";
import { changeNegative, changePositive } from "./metricChange.css";
import { periodComparisonNote } from "./PeriodComparisonNote.css";

export type PeriodComparisonNoteProps = {
  /** Inline styling for hover cards; default is a tab-panel footnote. */
  variant?: "footnote" | "inline";
  descriptionClass?: string;
};

const NOTE_PREFIX = "Percent change";

export default function PeriodComparisonNote(props: PeriodComparisonNoteProps) {
  const { comparisonNote } = useDashboardState();
  const className = () =>
    props.descriptionClass ??
    (props.variant === "inline" ? hoverCardDescription : periodComparisonNote);
  const suffix = () => {
    const note = comparisonNote();
    return note.startsWith(NOTE_PREFIX) ? note.slice(NOTE_PREFIX.length) : note;
  };

  return (
    <p class={className()}>
      {NOTE_PREFIX}{" "}
      <span class={changePositive} aria-hidden="true">
        ▲
      </span>{" "}
      <span class={changeNegative} aria-hidden="true">
        ▼
      </span>
      {suffix()}
    </p>
  );
}
