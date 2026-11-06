/// <reference types="solid-js" />
import { createEffect, createMemo, createSignal, For, type JSX, onMount, Show } from "solid-js";
import { formatRevenueAmount, formatSessionDuration } from "../../lib/dashboardApi";
import { scrollOpenedPanelIntoView } from "../../lib/journeyHistory";
import type {
  PaymentEntry,
  PaymentJourneyAction,
  PaymentJourneyResponse,
  PaymentLanding,
} from "../../lib/paymentsApi";
import { zonedDate } from "../../lib/timezone";
import type { VisitorEntry, VisitorJourneyResponse } from "../../lib/visitorsApi";
import { ensureWorldFlagsSpriteStylesheet } from "../../lib/worldFlagsSprite";
import { browserFamilyIconClass, deviceIconClass, osIconClass } from "./audienceLogic";
import {
  journeyAction,
  journeyActionBody,
  journeyActionIcon,
  journeyActionMeta,
  journeyActionMetadataTable,
  journeyActionSubtitle,
  journeyActionTime,
  journeyActionTitle,
  journeyColumns,
  journeyDayGroup,
  journeyDayHeader,
  journeyDetailLabel,
  journeyDetailRow,
  journeyDetailSectionHeader,
  journeyDetailTable,
  journeyDetailValue,
  journeyExternalLink,
  journeyHeader,
  journeyHeaderTitle,
  journeyIconButton,
  journeyIconButtonIcon,
  journeyShell,
  journeyStateMessage,
  journeySubtitle,
  journeySummaryPanel,
  journeyTimeline,
  journeyTimelineToggle,
  journeyTitle,
} from "./ConversionJourney.css";
import CountryLinkCell from "./CountryLinkCell";
import {
  formatPaidAt,
  formatTimeToPurchaseDuration,
  landingRevenueSourceTarget,
  landingSourceSubtitle,
  paymentCustomerLabel,
  paymentSourceSubtitle,
  paymentSubscriptionType,
  paymentTimeToPurchaseSeconds,
} from "./conversionsLogic";
import { useDashboardState } from "./dashboardPeriodContext";
import { gridGroupBadge, gridIconLabelCell } from "./grid.css";
import {
  journeyIconDiscoveredVia,
  journeyIconEvent,
  journeyIconGoal,
  journeyIconOutboundLink,
  journeyIconPageView,
  journeyIconPurchase,
} from "./icons.css";
import { errorMessage } from "./messages.css";
import {
  type RevenueSourceLinkTarget,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  type SourceGroup,
} from "./revenueSourcesLogic";
import ViewDetailsButton from "./ViewDetailsButton";

export type JourneyDetailProps = {
  payment?: PaymentEntry | undefined;
  visitor?: VisitorEntry | undefined;
  journey?: PaymentJourneyResponse | VisitorJourneyResponse | undefined;
  loading: boolean;
  error: string;
  backLabel?: string;
  titleFallback?: string;
  onBack: () => void;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
};

function formatTimeToPurchase(payment: PaymentEntry): string {
  const seconds = paymentTimeToPurchaseSeconds(payment);
  if (seconds === null) {
    return "—";
  }
  return formatTimeToPurchaseDuration(seconds);
}

function formatOptional(value: string | number | undefined | null): string {
  if (value === undefined || value === null || value === "") {
    return "—";
  }
  return String(value);
}

function formatMetadataValue(value: unknown): string {
  if (value === undefined || value === null) {
    return "—";
  }
  if (typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
}

function flattenMetadataEntries(
  metadata: Record<string, unknown> | undefined,
  prefix = "",
): [string, string][] {
  if (!metadata) {
    return [];
  }
  const rows: [string, string][] = [];
  for (const [key, value] of Object.entries(metadata)) {
    const label = prefix ? `${prefix}.${key}` : key;
    if (value !== null && typeof value === "object" && !Array.isArray(value)) {
      rows.push(...flattenMetadataEntries(value as Record<string, unknown>, label));
    } else {
      rows.push([label, formatMetadataValue(value)]);
    }
  }
  return rows;
}

function outboundDestinationHref(to: string): string | undefined {
  if (to.startsWith("mailto:")) {
    return undefined;
  }
  return `https://${to}`;
}

function renderOutboundDestinationLink(to: string): string | JSX.Element {
  const href = outboundDestinationHref(to);
  if (!href) {
    return to;
  }
  return (
    <a class={journeyExternalLink} href={href} target="_blank" rel="noopener noreferrer">
      {to}
    </a>
  );
}

function journeyActionMetadataRows(
  action: PaymentJourneyAction,
): [string, string | JSX.Element][] | undefined {
  const rows = flattenMetadataEntries(action.metadata);
  if (rows.length === 0) {
    return undefined;
  }
  if (action.kind === "event" && action.name === OUTBOUND_CLICK_EVENT_NAME) {
    return rows.map(([label, value]) => {
      if (label === "to") {
        return [label, renderOutboundDestinationLink(value)] as const;
      }
      return [label, value] as const;
    });
  }
  return rows;
}

function renderIconLabel(iconClass: string, label: string): JSX.Element {
  return (
    <span class={gridIconLabelCell}>
      <span class={iconClass} aria-hidden="true" />
      <span>{label}</span>
    </span>
  );
}

function renderCountryValue(
  country: string | undefined,
  onCountrySelect?: (countryCode: string) => void,
): string | JSX.Element {
  if (!country) {
    return "—";
  }
  return <CountryLinkCell country={country} onCountrySelect={onCountrySelect} />;
}

function renderBrowserValue(browserFamily: string | undefined): string | JSX.Element {
  if (!browserFamily) {
    return "—";
  }
  return renderIconLabel(browserFamilyIconClass(browserFamily), browserFamily);
}

function renderOsValue(os: string | undefined): string | JSX.Element {
  if (!os) {
    return "—";
  }
  return renderIconLabel(osIconClass(os), os);
}

type JourneyLanding = PaymentLanding | VisitorEntry | undefined;

function renderDeviceValue(landing: JourneyLanding): string | JSX.Element {
  if (!landing) {
    return "—";
  }
  return renderIconLabel(
    deviceIconClass(landing.device),
    `${landing.device} (${landing.viewport_width} x ${landing.viewport_height})`,
  );
}

type JourneyDetailRow = [string, string | JSX.Element];

const OUTBOUND_CLICK_EVENT_NAME = "revscope-outbound-click";

function JourneyKeyValueTable(props: {
  rows: readonly (readonly [string, string | JSX.Element])[];
  tableClass?: string;
  trailingSection?: {
    header: string;
    rows: readonly (readonly [string, string])[];
  };
}) {
  return (
    <table class={props.tableClass ?? journeyDetailTable}>
      <tbody>
        <For each={[...props.rows]}>
          {([label, value]) => (
            <tr class={journeyDetailRow}>
              <th class={journeyDetailLabel} scope="row">
                {label}
              </th>
              <td class={journeyDetailValue}>{value}</td>
            </tr>
          )}
        </For>
        <Show
          when={
            props.trailingSection && props.trailingSection.rows.length > 0
              ? props.trailingSection
              : undefined
          }
        >
          {(section) => (
            <>
              <tr>
                <th class={journeyDetailSectionHeader} colspan={2} scope="colgroup">
                  {section().header}
                </th>
              </tr>
              <For each={[...section().rows]}>
                {([label, value]) => (
                  <tr class={journeyDetailRow}>
                    <th class={journeyDetailLabel} scope="row">
                      {label}
                    </th>
                    <td class={journeyDetailValue}>{value}</td>
                  </tr>
                )}
              </For>
            </>
          )}
        </Show>
      </tbody>
    </table>
  );
}

function ordinalSuffix(day: number): string {
  if (day >= 11 && day <= 13) {
    return "th";
  }
  switch (day % 10) {
    case 1:
      return "st";
    case 2:
      return "nd";
    case 3:
      return "rd";
    default:
      return "th";
  }
}

function formatTimelineDay(unixSeconds: number, timeZone: string): string {
  const date = new Date(unixSeconds * 1000);
  const weekday = date.toLocaleDateString(undefined, {
    weekday: "long",
    timeZone,
  });
  const month = date.toLocaleDateString(undefined, {
    month: "long",
    timeZone,
  });
  // Intl names the weekday and month, but the ordinal suffix needs the number.
  const { day } = zonedDate(timeZone, unixSeconds);
  return `${weekday} ${day}${ordinalSuffix(day)} ${month}`;
}

function formatTimelineTime(unixSeconds: number, timeZone: string): string {
  return new Date(unixSeconds * 1000).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
    timeZone,
  });
}

function actionTitle(action: PaymentJourneyAction): string {
  switch (action.kind) {
    case "source":
      return "Discovered via";
    case "pageview":
      return "Page view";
    case "purchase":
      return "Purchase";
    case "event":
      return action.goal_id !== undefined
        ? action.name
          ? `Goal: ${action.name}`
          : "Goal"
        : action.name
          ? `Event: ${action.name}`
          : "Event";
  }
}

function actionIconClass(action: PaymentJourneyAction): string {
  switch (action.kind) {
    case "source":
      return journeyIconDiscoveredVia;
    case "pageview":
      return journeyIconPageView;
    case "event":
      if (action.name === OUTBOUND_CLICK_EVENT_NAME) {
        return journeyIconOutboundLink;
      }
      return action.goal_id !== undefined ? journeyIconGoal : journeyIconEvent;
    case "purchase":
      return journeyIconPurchase;
  }
}

function formatJourneyActionSubtitle(
  action: PaymentJourneyAction,
  landing: JourneyLanding,
): string | undefined {
  if (action.kind === "source") {
    return landingSourceSubtitle(landing);
  }
  return action.subtitle;
}

function campaignReferrerUrl(
  action: PaymentJourneyAction,
  landing: JourneyLanding,
): string | undefined {
  if (action.kind !== "source" || !landing?.campaign) {
    return undefined;
  }
  const referrer = landing.referrer?.referrer;
  return referrer && referrer !== "" ? referrer : undefined;
}

function findLandingPageview(
  timeline: PaymentJourneyAction[] | undefined,
): PaymentJourneyAction | undefined {
  return timeline?.find((action) => action.kind === "pageview");
}

function JourneyActionSubtitle(props: {
  action: PaymentJourneyAction;
  landing: JourneyLanding;
  landingPageview?: PaymentJourneyAction;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
}) {
  const subtitle = () => formatJourneyActionSubtitle(props.action, props.landing);
  const referrerUrl = () => campaignReferrerUrl(props.action, props.landing);
  const revenueSourceTarget = () =>
    props.action.kind === "source" ? landingRevenueSourceTarget(props.landing) : null;
  const pageLink = () => {
    if (props.action.kind !== "pageview" || props.action.path_id === undefined) {
      return null;
    }
    return {
      pathId: props.action.path_id,
      isLanding: props.action === props.landingPageview,
    };
  };
  return (
    <Show when={subtitle()}>
      {(text) => (
        <p class={journeyActionSubtitle}>
          <Show when={props.action.source_group}>
            {(group) => (
              <>
                <span class={gridGroupBadge} style={groupBadgeStyle(group())}>
                  {SOURCE_GROUP_LABELS[group()]}
                </span>{" "}
              </>
            )}
          </Show>
          <Show
            when={pageLink()}
            fallback={
              <Show when={revenueSourceTarget()} fallback={text()}>
                {(target) => (
                  <ViewDetailsButton
                    title={`View details for ${text()} in Sources`}
                    onClick={() => props.onRevenueSourceSelect?.(target())}
                  >
                    {text()}
                  </ViewDetailsButton>
                )}
              </Show>
            }
          >
            {(link) => (
              <ViewDetailsButton
                title={`View details for ${text()} in ${link().isLanding ? "Landing Pages" : "Pages"}`}
                onClick={() => {
                  if (link().isLanding) {
                    props.onLandingPathSelect?.(link().pathId);
                  } else {
                    props.onPathSelect?.(link().pathId);
                  }
                }}
              >
                {text()}
              </ViewDetailsButton>
            )}
          </Show>
          <Show when={referrerUrl()}>{(url) => <> · {url()}</>}</Show>
        </p>
      )}
    </Show>
  );
}

function buildActionMeta(action: PaymentJourneyAction): string[] {
  const meta: string[] = [];
  if (action.duration_seconds !== undefined) {
    meta.push(`Duration ${formatSessionDuration(action.duration_seconds)}`);
  }
  if (action.scrolled_percent !== undefined) {
    meta.push(`Scrolled ${action.scrolled_percent}%`);
  }
  return meta;
}

function groupBadgeStyle(group: SourceGroup): {
  background: string;
  color: string;
} {
  return {
    background: SOURCE_GROUP_COLORS[group],
    color: "#ffffff",
  };
}

type TimelineDayGroup = {
  day: string;
  actions: PaymentJourneyAction[];
};

function groupTimelineByDay(actions: PaymentJourneyAction[], timeZone: string): TimelineDayGroup[] {
  const groups: TimelineDayGroup[] = [];
  for (const action of actions) {
    const day = formatTimelineDay(action.occurred_at, timeZone);
    const current = groups[groups.length - 1];
    if (current?.day === day) {
      current.actions.push(action);
    } else {
      groups.push({ day, actions: [action] });
    }
  }
  return groups;
}

function splitTimelineAtPurchase(timeline: PaymentJourneyAction[]): {
  beforePurchase: PaymentJourneyAction[];
  afterPurchase: PaymentJourneyAction[];
} {
  const purchaseIndex = timeline.findLastIndex((action) => action.kind === "purchase");
  if (purchaseIndex === -1) {
    return { beforePurchase: timeline, afterPurchase: [] };
  }
  return {
    beforePurchase: timeline.slice(0, purchaseIndex + 1),
    afterPurchase: timeline.slice(purchaseIndex + 1),
  };
}

function JourneyTimelineAction(props: {
  action: PaymentJourneyAction;
  landing: JourneyLanding;
  landingPageview?: PaymentJourneyAction;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
}) {
  const { timeZone } = useDashboardState();
  const meta = buildActionMeta(props.action);
  return (
    <article class={journeyAction}>
      <span class={journeyActionIcon}>
        <span class={actionIconClass(props.action)} aria-hidden="true" />
      </span>
      <div class={journeyActionBody}>
        <div class={journeyActionTitle}>
          <span>{actionTitle(props.action)}</span>
          <time class={journeyActionTime}>
            {formatTimelineTime(props.action.occurred_at, timeZone())}
          </time>
        </div>
        <JourneyActionSubtitle
          action={props.action}
          landing={props.landing}
          landingPageview={props.landingPageview}
          onRevenueSourceSelect={props.onRevenueSourceSelect}
          onPathSelect={props.onPathSelect}
          onLandingPathSelect={props.onLandingPathSelect}
        />
        <Show when={meta.length > 0}>
          <p class={journeyActionMeta}>{meta.join(" · ")}</p>
        </Show>
        <Show when={journeyActionMetadataRows(props.action)}>
          {(metadataRows) => (
            <JourneyKeyValueTable tableClass={journeyActionMetadataTable} rows={metadataRows()} />
          )}
        </Show>
      </div>
    </article>
  );
}

function JourneyTimelineDayGroups(props: {
  groups: TimelineDayGroup[];
  landing: JourneyLanding;
  landingPageview?: PaymentJourneyAction;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
}) {
  return (
    <For each={props.groups}>
      {(group) => (
        <section class={journeyDayGroup}>
          <h3 class={journeyDayHeader}>{group.day}</h3>
          <For each={group.actions}>
            {(action) => (
              <JourneyTimelineAction
                action={action}
                landing={props.landing}
                landingPageview={props.landingPageview}
                onRevenueSourceSelect={props.onRevenueSourceSelect}
                onPathSelect={props.onPathSelect}
                onLandingPathSelect={props.onLandingPathSelect}
              />
            )}
          </For>
        </section>
      )}
    </For>
  );
}

function JourneyTimelineToggleButton(props: {
  expanded: boolean;
  hiddenCount: number;
  onClick: () => void;
}) {
  return (
    <div class={journeyTimelineToggle}>
      <button
        type="button"
        class={journeyIconButton}
        aria-expanded={props.expanded}
        aria-label={
          props.expanded
            ? "Hide events after purchase"
            : `Show ${props.hiddenCount} event${props.hiddenCount === 1 ? "" : "s"} after purchase`
        }
        onClick={props.onClick}
      >
        <svg
          class={journeyIconButtonIcon}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <Show when={props.expanded} fallback={<path d="M6 9l6 6 6-6" />}>
            <path d="M18 15l-6-6-6 6" />
          </Show>
        </svg>
      </button>
    </div>
  );
}

export default function JourneyDetail(props: JourneyDetailProps) {
  const { timeZone } = useDashboardState();
  let journeyRoot: HTMLElement | undefined;
  let revealedFor: string | undefined;

  onMount(() => {
    ensureWorldFlagsSpriteStylesheet();
  });

  const [postPurchaseExpanded, setPostPurchaseExpanded] = createSignal(false);

  createEffect(() => {
    const currentJourney = props.journey;
    if (currentJourney && "payment" in currentJourney) {
      currentJourney.payment.id;
    } else if (currentJourney && "visitor" in currentJourney) {
      currentJourney.visitor.visitor_timestamp_id;
    }
    setPostPurchaseExpanded(false);
  });

  const payment = () => {
    const currentJourney = props.journey;
    if (currentJourney && "payment" in currentJourney) {
      return currentJourney.payment;
    }
    return props.payment;
  };
  const visitor = () => {
    const currentJourney = props.journey;
    if (currentJourney && "visitor" in currentJourney) {
      return currentJourney.visitor;
    }
    return props.visitor;
  };
  const landing = (): JourneyLanding => payment()?.landing ?? visitor();
  const subject = () => payment() ?? visitor();
  const detailRows = createMemo((): JourneyDetailRow[] => {
    const currentPayment = payment();
    const currentLanding = landing();
    const currentVisitor = visitor();
    if (!currentPayment && !currentVisitor) {
      return [];
    }
    if (!currentPayment && currentVisitor) {
      return [
        ["Last view at", formatPaidAt(currentVisitor.last_view_at, timeZone())],
        [
          "Session age",
          formatTimeToPurchaseDuration(
            Math.max(0, currentVisitor.last_view_at - currentVisitor.viewed_at),
          ),
        ],
        ["Country", renderCountryValue(currentLanding?.country, props.onCountrySelect)],
        ["City", formatOptional(currentLanding?.city)],
        ["Active duration", formatSessionDuration(currentVisitor.session_duration_seconds)],
        ["Session pages", formatOptional(currentVisitor.session_page_count)],
        ["Browser", renderBrowserValue(currentLanding?.browser_family)],
        ["OS", renderOsValue(currentLanding?.os)],
        ["Device", renderDeviceValue(currentLanding)],
      ];
    }
    if (!currentPayment) {
      return [];
    }
    const rows: JourneyDetailRow[] = [
      ["Paid at", formatPaidAt(currentPayment.paid_at, timeZone())],
      ["Time to purchase", formatTimeToPurchase(currentPayment)],
      ["Amount", formatRevenueAmount(currentPayment.amount)],
      ["Purchase currency", currentPayment.currency],
      ["Subscription", paymentSubscriptionType(currentPayment)],
      [
        "MRR amount",
        currentPayment.mrr_amount !== undefined
          ? formatRevenueAmount(currentPayment.mrr_amount)
          : "—",
      ],
      ["Country", renderCountryValue(currentLanding?.country, props.onCountrySelect)],
      ["City", formatOptional(currentLanding?.city)],
      [
        "Active duration",
        currentLanding ? formatSessionDuration(currentLanding.session_duration_seconds) : "—",
      ],
      ["Session pages", formatOptional(currentLanding?.session_page_count)],
      ["Browser", renderBrowserValue(currentLanding?.browser_family)],
      ["OS", renderOsValue(currentLanding?.os)],
      ["Device", renderDeviceValue(currentLanding)],
      ["Payment ID", currentPayment.provider_payment_id],
      ["Order ID", formatOptional(currentPayment.provider_order_id)],
      ["Invoice ID", formatOptional(currentPayment.provider_invoice_id)],
      ["Customer ID", formatOptional(currentPayment.provider_customer_id)],
      ["Subscription ID", formatOptional(currentPayment.provider_subscription_id)],
    ];
    return rows;
  });
  const customAttributeRows = createMemo(() => flattenMetadataEntries(payment()?.metadata));
  const timelineSplit = createMemo(() => {
    const { beforePurchase, afterPurchase } = splitTimelineAtPurchase(
      props.journey?.timeline ?? [],
    );
    return {
      beforePurchaseGroups: groupTimelineByDay(beforePurchase, timeZone()),
      afterPurchaseGroups: groupTimelineByDay(afterPurchase, timeZone()),
      afterPurchaseCount: afterPurchase.length,
    };
  });
  const landingPageview = createMemo(() => findLandingPageview(props.journey?.timeline));

  createEffect(() => {
    const currentVisitor = visitor();
    const currentPayment = payment();
    const openedId = currentVisitor
      ? `v:${currentVisitor.visitor_timestamp_id}`
      : currentPayment
        ? `p:${currentPayment.id}`
        : undefined;
    if (!openedId) {
      revealedFor = undefined;
      return;
    }
    if (openedId === revealedFor) {
      return;
    }
    revealedFor = openedId;
    if (journeyRoot) {
      scrollOpenedPanelIntoView(journeyRoot);
    }
  });

  return (
    <section class={journeyShell} ref={journeyRoot} aria-live="polite">
      <div class={journeyHeader}>
        <button
          type="button"
          class={journeyIconButton}
          aria-label={props.backLabel ?? "Back to conversions"}
          onClick={props.onBack}
        >
          <svg
            class={journeyIconButtonIcon}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M19 12H5" />
            <path d="M12 19l-7-7 7-7" />
          </svg>
        </button>
        <div class={journeyHeaderTitle}>
          <h2 class={journeyTitle}>
            {(() => {
              const currentPayment = payment();
              if (currentPayment) {
                return paymentCustomerLabel(currentPayment);
              }
              const currentVisitor = visitor();
              if (currentVisitor) {
                return `Visitor ${currentVisitor.visitor_timestamp_id}`;
              }
              return props.titleFallback ?? "Customer journey";
            })()}
          </h2>
          <p class={journeySubtitle}>
            {(() => {
              const currentPayment = payment();
              if (currentPayment) {
                return paymentSourceSubtitle(currentPayment);
              }
              const currentVisitor = visitor();
              if (currentVisitor) {
                return landingSourceSubtitle(currentVisitor);
              }
              return "Loading";
            })()}
          </p>
        </div>
        <button type="button" class={journeyIconButton} aria-label="Close" onClick={props.onBack}>
          <svg
            class={journeyIconButtonIcon}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </div>

      <Show when={subject()} fallback={<p class={journeyStateMessage}>Loading…</p>}>
        <div class={journeyColumns}>
          <aside class={journeySummaryPanel}>
            <JourneyKeyValueTable
              rows={detailRows()}
              trailingSection={{
                header: "Custom Attributes",
                rows: customAttributeRows(),
              }}
            />
          </aside>

          <div class={journeyTimeline}>
            <Show when={props.loading}>
              <p class={journeyStateMessage}>Loading journey…</p>
            </Show>
            <Show when={props.error}>
              <p class={errorMessage}>{props.error}</p>
            </Show>
            <Show when={!props.loading && !props.error}>
              <JourneyTimelineDayGroups
                groups={timelineSplit().beforePurchaseGroups}
                landing={landing()}
                landingPageview={landingPageview()}
                onRevenueSourceSelect={props.onRevenueSourceSelect}
                onPathSelect={props.onPathSelect}
                onLandingPathSelect={props.onLandingPathSelect}
              />
              <Show when={timelineSplit().afterPurchaseCount > 0}>
                <Show
                  when={postPurchaseExpanded()}
                  fallback={
                    <JourneyTimelineToggleButton
                      expanded={false}
                      hiddenCount={timelineSplit().afterPurchaseCount}
                      onClick={() => setPostPurchaseExpanded(true)}
                    />
                  }
                >
                  <JourneyTimelineDayGroups
                    groups={timelineSplit().afterPurchaseGroups}
                    landing={landing()}
                    landingPageview={landingPageview()}
                    onRevenueSourceSelect={props.onRevenueSourceSelect}
                    onPathSelect={props.onPathSelect}
                    onLandingPathSelect={props.onLandingPathSelect}
                  />
                  <JourneyTimelineToggleButton
                    expanded
                    hiddenCount={timelineSplit().afterPurchaseCount}
                    onClick={() => setPostPurchaseExpanded(false)}
                  />
                </Show>
              </Show>
            </Show>
          </div>
        </div>
      </Show>
    </section>
  );
}
