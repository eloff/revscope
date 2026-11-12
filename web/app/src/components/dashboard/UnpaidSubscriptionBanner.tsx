/// <reference types="solid-js" />
import type { UnpaidSubscription } from "@revscope/web-shared/authState";
import {
  formatDate,
  unpaidAmountLabel,
  unpaidPlanLabel,
} from "@revscope/web-shared/subscriptionLabels";
import { Show } from "solid-js";
import { banner, bannerText, payLink } from "./UnpaidSubscriptionBanner.css";

type UnpaidSubscriptionBannerProps = {
  unpaid: UnpaidSubscription | null | undefined;
  /** What the free plan shows, in months back from today. */
  retentionMonths: number | null | undefined;
};

/** Says the account was moved onto the free plan for want of payment, and links to paying. */
export default function UnpaidSubscriptionBanner(props: UnpaidSubscriptionBannerProps) {
  return (
    <Show when={props.unpaid}>
      {(unpaid) => (
        <div class={banner} role="status">
          <p class={bannerText}>
            Your {unpaidPlanLabel(unpaid())} subscription wasn't paid for, so this account moved to
            the Free plan on {formatDate(unpaid().ended_at)}.
            <Show when={props.retentionMonths}>
              {(months) => (
                <>
                  {" "}
                  The dashboard shows the last {months()} months. Older data is kept, and as much as
                  your plan keeps shows again when you restore it.
                </>
              )}
            </Show>
          </p>
          <a class={payLink} href="/billing">
            Pay {unpaidAmountLabel(unpaid())} to restore it
          </a>
        </div>
      )}
    </Show>
  );
}
