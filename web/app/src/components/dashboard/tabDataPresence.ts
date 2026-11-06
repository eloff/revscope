import type { AudienceResponse } from "../../lib/audienceApi";
import type { CrawlersResponse } from "../../lib/crawlersApi";
import { type OverviewResponse, sumOverviewBuckets } from "../../lib/dashboardApi";
import type { EventsResponse } from "../../lib/eventsApi";
import type { PagesResponse } from "../../lib/pagesApi";
import type { PaymentActivityResponse } from "../../lib/paymentActivityApi";
import type { ConversionsResponse } from "../../lib/paymentsApi";
import type { RevenueSourceResponse } from "../../lib/revenueSourcesApi";
import type { SearchTermsResponse } from "../../lib/searchTermsApi";
import type { VisitorsResponse } from "../../lib/visitorsApi";

/**
 * Whether a tab's response holds anything worth rendering. A project that is
 * still being set up, or a date range before the tracking script went live,
 * answers no for every tab and gets the setup prompt instead.
 */

export function hasOverviewData(data: OverviewResponse): boolean {
  const totals = sumOverviewBuckets(data.buckets);
  return (
    totals.visitors > 0 ||
    totals.converting_visitors > 0 ||
    data.current.total_revenue > 0 ||
    data.mrr > 0
  );
}

export function hasPaymentActivityData(data: PaymentActivityResponse): boolean {
  return data.days.length > 0 || data.monthly_revenue.some((month) => month.revenue !== 0);
}

export function hasRevenueSourcesData(data: RevenueSourceResponse): boolean {
  return data.current.entries.length > 0;
}

export function hasConversionsData(data: ConversionsResponse): boolean {
  return data.conversions.length > 0;
}

export function hasSearchTermsData(data: SearchTermsResponse): boolean {
  return data.current.terms.length > 0;
}

export function hasAudienceData(data: AudienceResponse): boolean {
  return (
    data.countries.current.length > 0 ||
    data.cities.current.length > 0 ||
    data.browsers.current.length > 0 ||
    data.os.current.length > 0 ||
    data.devices.current.length > 0 ||
    data.screen_sizes.current.length > 0 ||
    data.viewports.current.length > 0
  );
}

export function hasPagesData(data: PagesResponse): boolean {
  return data.pages.length > 0;
}

export function hasVisitorsData(data: VisitorsResponse): boolean {
  return data.visitors.length > 0;
}

export function hasEventsData(data: EventsResponse): boolean {
  return data.events.length > 0;
}

export function hasCrawlersData(data: CrawlersResponse): boolean {
  return data.rows.length > 0 || Object.values(data.category_totals).some((views) => views > 0);
}
