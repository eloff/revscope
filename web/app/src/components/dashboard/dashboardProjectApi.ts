import type { AuthSession } from "@revscope/web-shared/authState";
import { fetchAudience } from "../../lib/audienceApi";
import { fetchCrawlerDetail, fetchCrawlers } from "../../lib/crawlersApi";
import { fetchOverview } from "../../lib/dashboardApi";
import { fetchEvents } from "../../lib/eventsApi";
import { fetchLandingPages, fetchPages, fetchPageTransitions } from "../../lib/pagesApi";
import { fetchPaymentActivity } from "../../lib/paymentActivityApi";
import { fetchConversions, fetchPaymentJourney, fetchPayments } from "../../lib/paymentsApi";
import { fetchRevenueSources } from "../../lib/revenueSourcesApi";
import { fetchSearchTerms } from "../../lib/searchTermsApi";
import { fetchVisitorJourney, fetchVisitors } from "../../lib/visitorsApi";

export type ProjectAccess = {
  projectId: () => string;
  publicAccess: () => boolean;
  /** Throws when nobody is signed in; only called for non-public access. */
  session: () => AuthSession;
};

/**
 * Binds a read endpoint to the project, reading anonymously for public
 * access (the demo) and as the signed-in user otherwise.
 */
function withAccess<A extends unknown[], R>(
  access: ProjectAccess,
  read: (session: AuthSession | undefined, projectId: string, ...args: A) => Promise<R>,
): (...args: A) => Promise<R> {
  // async so a missing session rejects like a failed request instead of throwing.
  return async (...args) =>
    read(access.publicAccess() ? undefined : access.session(), access.projectId(), ...args);
}

export type DashboardProjectApi = ReturnType<typeof createDashboardProjectApi>;

/** The dashboard's endpoints bound to one project and the viewer's access to it. */
export function createDashboardProjectApi(access: ProjectAccess) {
  return {
    overview: withAccess(access, fetchOverview),
    revenueSources: withAccess(access, fetchRevenueSources),
    paymentActivity: withAccess(access, fetchPaymentActivity),
    searchTerms: withAccess(access, fetchSearchTerms),
    audience: withAccess(access, fetchAudience),
    pages: withAccess(access, fetchPages),
    landingPages: withAccess(access, fetchLandingPages),
    pageTransitions: withAccess(access, fetchPageTransitions),
    events: withAccess(access, fetchEvents),
    crawlers: withAccess(access, fetchCrawlers),
    crawlerDetail: withAccess(access, fetchCrawlerDetail),
    visitors: withAccess(access, fetchVisitors),
    visitorJourney: withAccess(access, fetchVisitorJourney),
    conversions: withAccess(access, fetchConversions),
    payments: withAccess(access, fetchPayments),
    paymentJourney: withAccess(access, fetchPaymentJourney),
  };
}
