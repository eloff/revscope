/// <reference types="solid-js" />

import { createMemo, For, onMount, Show } from "solid-js";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import type { PaymentEntry } from "../../lib/paymentsApi";
import { ensureWorldFlagsSpriteStylesheet } from "../../lib/worldFlagsSprite";
import CountryLinkCell from "./CountryLinkCell";
import { nextColumnSort } from "./columnSort";
import {
  type ConversionsSortColumn,
  conversionsSortValue,
  formatPaidAt,
  formatTimeToPurchaseDuration,
  paymentCustomerLabel,
  paymentSubscriptionType,
  paymentTimeToPurchaseSeconds,
} from "./conversionsLogic";
import { useDashboardState } from "./dashboardPeriodContext";
import { clickableRow, gridNumberCell, gridReferrerCell, gridShell, gridTable } from "./grid.css";
import PaginationBar from "./PaginationBar";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import SortableHeaderCell from "./SortableHeaderCell";
import SourceLinkCell from "./SourceLinkCell";

export type PaymentsTableProps = {
  payments: PaymentEntry[] | undefined;
  loading: boolean;
  emptyLabel: string;
  loadingLabel?: string;
  /** Absolute row offset of the current page (for "Showing X–Y"). */
  offset?: number;
  /** When set with onPageChange, enables Previous/Next controls. */
  hasNextPage?: boolean;
  onPageChange?: (direction: "previous" | "next") => void;
  countLabel?: (count: number, offset: number) => string;
  onPaymentSelect?: (payment: PaymentEntry) => void;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onCountrySelect?: (countryCode: string) => void;
};

const COLUMNS: {
  key: ConversionsSortColumn;
  label: string;
  numeric: boolean;
}[] = [
  { key: "customer", label: "Customer", numeric: false },
  { key: "source", label: "Source", numeric: false },
  { key: "amount", label: "Amount", numeric: true },
  { key: "subscriptionType", label: "Subscription type", numeric: false },
  { key: "purchasedAt", label: "Purchased at", numeric: true },
  { key: "timeToPurchase", label: "Time to purchase", numeric: true },
];

const RIGHT_ALIGNED_COLUMNS = new Set<ConversionsSortColumn>([
  "amount",
  "purchasedAt",
  "timeToPurchase",
]);

function formatTimeToPurchase(payment: PaymentEntry): string {
  const seconds = paymentTimeToPurchaseSeconds(payment);
  if (seconds === null) {
    return "—";
  }
  return formatTimeToPurchaseDuration(seconds);
}

const TABLE_COLUMN_COUNT = COLUMNS.length + 1;

export default function PaymentsTable(props: PaymentsTableProps) {
  const { timeZone } = useDashboardState();

  onMount(() => {
    ensureWorldFlagsSpriteStylesheet();
  });

  const { conversionsSort, setConversionsSort } = useDashboardPreferences();

  const handleSort = (column: ConversionsSortColumn) => {
    setConversionsSort((previous) => nextColumnSort(previous, column));
  };

  const sortedPayments = createMemo(() => {
    const payments = props.payments ?? [];
    const state = conversionsSort();
    const direction = state.direction === "asc" ? 1 : -1;
    return [...payments].sort((left, right) => {
      const leftValue = conversionsSortValue(left, state.column);
      const rightValue = conversionsSortValue(right, state.column);
      let comparison: number;
      if (typeof leftValue === "string" && typeof rightValue === "string") {
        comparison = leftValue.localeCompare(rightValue);
      } else {
        comparison = (leftValue as number) - (rightValue as number);
      }
      if (comparison !== 0) {
        return comparison * direction;
      }
      if (left.paid_at !== right.paid_at) {
        return right.paid_at - left.paid_at;
      }
      return left.id - right.id;
    });
  });

  const showInitialLoading = () => props.loading && !props.payments;
  const paginationEnabled = () => props.onPageChange !== undefined;
  const offset = () => props.offset ?? 0;

  const countLabel = () => {
    const count = props.payments?.length ?? 0;
    if (count === 0) {
      return props.emptyLabel;
    }
    if (props.countLabel) {
      return props.countLabel(count, offset());
    }
    if (paginationEnabled()) {
      const start = offset() + 1;
      const end = offset() + count;
      return `Showing ${start.toLocaleString()}-${end.toLocaleString()} conversions`;
    }
    return `${count.toLocaleString()} conversion${count === 1 ? "" : "s"}`;
  };

  return (
    <div class={gridShell}>
      <table class={gridTable}>
        <thead>
          <tr>
            <th>Location</th>
            <For each={COLUMNS}>
              {(column) => (
                <SortableHeaderCell
                  label={column.label}
                  sortDirection={
                    conversionsSort().column === column.key
                      ? conversionsSort().direction
                      : undefined
                  }
                  numeric={RIGHT_ALIGNED_COLUMNS.has(column.key)}
                  onSort={() => handleSort(column.key)}
                />
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <Show when={!showInitialLoading()}>
            <For each={sortedPayments()}>
              {(payment) => (
                <tr
                  class={props.onPaymentSelect ? clickableRow : undefined}
                  role={props.onPaymentSelect ? "button" : undefined}
                  tabIndex={props.onPaymentSelect ? 0 : undefined}
                  aria-label={
                    props.onPaymentSelect
                      ? `View journey for ${paymentCustomerLabel(payment)}`
                      : undefined
                  }
                  onClick={() => props.onPaymentSelect?.(payment)}
                  onKeyDown={(event) => {
                    if (props.onPaymentSelect && (event.key === "Enter" || event.key === " ")) {
                      event.preventDefault();
                      props.onPaymentSelect(payment);
                    }
                  }}
                >
                  <td>
                    <CountryLinkCell
                      country={payment.landing?.country}
                      onCountrySelect={props.onCountrySelect}
                    />
                  </td>
                  <td class={gridReferrerCell}>{paymentCustomerLabel(payment)}</td>
                  <td class={gridReferrerCell}>
                    <SourceLinkCell
                      landing={payment.landing}
                      onRevenueSourceSelect={props.onRevenueSourceSelect}
                    />
                  </td>
                  <td class={gridNumberCell}>{formatRevenueAmount(payment.amount)}</td>
                  <td>{paymentSubscriptionType(payment)}</td>
                  <td class={gridNumberCell}>{formatPaidAt(payment.paid_at, timeZone())}</td>
                  <td class={gridNumberCell}>{formatTimeToPurchase(payment)}</td>
                </tr>
              )}
            </For>
          </Show>
          <Show when={showInitialLoading()}>
            <tr>
              <td colSpan={TABLE_COLUMN_COUNT}>{props.loadingLabel ?? "Loading…"}</td>
            </tr>
          </Show>
        </tbody>
      </table>
      <PaginationBar
        label={countLabel()}
        loading={props.loading}
        showButtons={paginationEnabled()}
        atStart={offset() <= 0}
        atEnd={!props.hasNextPage}
        onPrevious={() => props.onPageChange?.("previous")}
        onNext={() => props.onPageChange?.("next")}
      />
    </div>
  );
}
