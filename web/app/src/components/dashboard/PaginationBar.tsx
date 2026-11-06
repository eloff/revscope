import { Show } from "solid-js";
import { paginationBar, paginationButton, paginationLabel } from "./pagination.css";

interface PaginationBarProps {
  label: string;
  loading: boolean;
  atStart: boolean;
  atEnd: boolean;
  onPrevious: () => void;
  onNext: () => void;
  /** False hides the buttons, e.g. when every row fits on one page. */
  showButtons?: boolean;
}

/** Row count label with Previous/Next buttons under a grid. */
export default function PaginationBar(props: PaginationBarProps) {
  return (
    <div class={paginationBar}>
      <span class={paginationLabel}>{props.label}</span>
      <Show when={props.showButtons !== false}>
        <button
          type="button"
          class={paginationButton}
          disabled={props.loading || props.atStart}
          onClick={() => props.onPrevious()}
        >
          Previous
        </button>
        <button
          type="button"
          class={paginationButton}
          disabled={props.loading || props.atEnd}
          onClick={() => props.onNext()}
        >
          Next
        </button>
      </Show>
    </div>
  );
}
