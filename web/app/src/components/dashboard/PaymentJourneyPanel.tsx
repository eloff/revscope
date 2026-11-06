/// <reference types="solid-js" />
import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import {
  backFromJourneyHistory,
  clearJourneyHistoryParam,
  installJourneyHistoryListener,
  pushJourneyHistoryParam,
} from "../../lib/journeyHistory";
import type { PaymentEntry, PaymentJourneyResponse } from "../../lib/paymentsApi";
import JourneyDetail from "./ConversionJourney";
import PaymentsTable from "./PaymentsTable";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import SlidePanels from "./SlidePanels";

const PAYMENT_JOURNEY_PARAM = "payment_id";

export type PaymentJourneyPanelProps = {
  payments: PaymentEntry[] | undefined;
  loading: boolean;
  emptyLabel: string;
  loadingLabel?: string;
  offset?: number;
  hasNextPage?: boolean;
  onPageChange?: (direction: "previous" | "next") => void;
  countLabel?: (count: number, offset: number) => string;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
};

export default function PaymentJourneyPanel(props: PaymentJourneyPanelProps) {
  const [selectedPayment, setSelectedPayment] = createSignal<PaymentEntry | undefined>();
  const [journey, setJourney] = createSignal<PaymentJourneyResponse>();
  const [journeyLoading, setJourneyLoading] = createSignal(false);
  const [journeyError, setJourneyError] = createSignal("");
  let journeyRequestId = 0;

  const resetJourney = () => {
    journeyRequestId += 1;
    setSelectedPayment(undefined);
    setJourney(undefined);
    setJourneyLoading(false);
    setJourneyError("");
  };

  const closeJourney = () => {
    resetJourney();
    clearJourneyHistoryParam(PAYMENT_JOURNEY_PARAM);
  };

  const handleBack = () => {
    if (selectedPayment()) {
      backFromJourneyHistory();
      return;
    }
    closeJourney();
  };

  const openJourney = async (payment: PaymentEntry, options?: { fromHistory?: boolean }) => {
    setSelectedPayment(payment);
    setJourneyError("");
    if (!options?.fromHistory) {
      pushJourneyHistoryParam(PAYMENT_JOURNEY_PARAM, String(payment.id));
    }
    const requestId = ++journeyRequestId;
    setJourney(undefined);
    setJourneyLoading(true);
    try {
      const response = await props.fetchJourney(payment.id);
      if (requestId === journeyRequestId) {
        setJourney(response);
      }
    } catch (loadError) {
      if (requestId === journeyRequestId) {
        setJourneyError(
          loadError instanceof Error ? loadError.message : "Unable to load customer journey.",
        );
      }
    } finally {
      if (requestId === journeyRequestId) {
        setJourneyLoading(false);
      }
    }
  };

  onMount(() => {
    onCleanup(
      installJourneyHistoryListener(PAYMENT_JOURNEY_PARAM, (paymentId) => {
        if (!paymentId) {
          resetJourney();
          return;
        }
        const id = Number(paymentId);
        if (!Number.isInteger(id) || id <= 0) {
          resetJourney();
          return;
        }
        if (selectedPayment()?.id === id) {
          return;
        }
        const payment = props.payments?.find((entry) => entry.id === id);
        if (payment) {
          void openJourney(payment, { fromHistory: true });
          return;
        }
        resetJourney();
      }),
    );
  });

  createEffect(() => {
    const payments = props.payments;
    const selected = selectedPayment();
    if (!payments || !selected) {
      return;
    }
    if (!payments.some((payment) => payment.id === selected.id)) {
      closeJourney();
    }
  });

  return (
    <SlidePanels
      speed="fast"
      showSecond={selectedPayment() !== undefined}
      first={
        <PaymentsTable
          payments={props.payments}
          loading={props.loading}
          emptyLabel={props.emptyLabel}
          loadingLabel={props.loadingLabel}
          offset={props.offset}
          hasNextPage={props.hasNextPage}
          onPageChange={props.onPageChange}
          countLabel={props.countLabel}
          onPaymentSelect={(payment) => void openJourney(payment)}
          onRevenueSourceSelect={props.onRevenueSourceSelect}
          onCountrySelect={props.onCountrySelect}
        />
      }
      second={
        <JourneyDetail
          payment={selectedPayment()}
          journey={journey()}
          loading={journeyLoading()}
          error={journeyError()}
          onBack={handleBack}
          onRevenueSourceSelect={props.onRevenueSourceSelect}
          onPathSelect={props.onPathSelect}
          onLandingPathSelect={props.onLandingPathSelect}
          onCountrySelect={props.onCountrySelect}
        />
      }
    />
  );
}
