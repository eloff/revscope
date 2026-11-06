/// <reference types="solid-js" />
import { Show } from "solid-js";
import type { ConversionsResponse, PaymentJourneyResponse } from "../../lib/paymentsApi";
import { errorMessage } from "./messages.css";
import PaymentJourneyPanel from "./PaymentJourneyPanel";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";

export type ConversionsProps = {
  data: ConversionsResponse | undefined;
  loading: boolean;
  error: string;
  offset: number;
  onPageChange: (direction: "previous" | "next") => void;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
};

export default function Conversions(props: ConversionsProps) {
  return (
    <Show when={!props.error} fallback={<p class={errorMessage}>{props.error}</p>}>
      <PaymentJourneyPanel
        payments={props.data?.conversions}
        loading={props.loading}
        emptyLabel="No conversions for this period"
        offset={props.offset}
        hasNextPage={props.data?.next_cursor !== undefined}
        onPageChange={props.onPageChange}
        fetchJourney={props.fetchJourney}
        onRevenueSourceSelect={props.onRevenueSourceSelect}
        onPathSelect={props.onPathSelect}
        onLandingPathSelect={props.onLandingPathSelect}
        onCountrySelect={props.onCountrySelect}
      />
    </Show>
  );
}
