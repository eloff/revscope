import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { type CSSProperties, style } from "@vanilla-extract/css";

// SVG sources live under src/assets/icons. We read them at vanilla-extract
// compile time so background-image URLs are inlined into CSS (no runtime .svg
// requests). Vite's ?inline imports are not usable from .css.ts with the
// current @vanilla-extract/compiler vite-node pipeline.

const iconsDir = join(dirname(fileURLToPath(import.meta.url)), "../../assets/icons");

function loadInlineSvgDataUrl(relativePath: string): string {
  const svg = readFileSync(join(iconsDir, relativePath), "utf8");
  return `data:image/svg+xml,${encodeURIComponent(svg.trim())}`;
}

const unknownIcon = loadInlineSvgDataUrl("unknown.svg");
const baiduIcon = loadInlineSvgDataUrl("browsers/baidu.svg");
const braveIcon = loadInlineSvgDataUrl("browsers/brave.svg");
const chromeIcon = loadInlineSvgDataUrl("browsers/chrome.svg");
const duckduckgoIcon = loadInlineSvgDataUrl("browsers/duckduckgo.svg");
const edgeIcon = loadInlineSvgDataUrl("browsers/edge.svg");
const firefoxIcon = loadInlineSvgDataUrl("browsers/firefox.svg");
const ieIcon = loadInlineSvgDataUrl("browsers/ie.svg");
const kaiIcon = loadInlineSvgDataUrl("browsers/kai.svg");
const operaIcon = loadInlineSvgDataUrl("browsers/opera.svg");
const qqIcon = loadInlineSvgDataUrl("browsers/qq.svg");
const safariIcon = loadInlineSvgDataUrl("browsers/safari.svg");
const samsungIcon = loadInlineSvgDataUrl("browsers/samsung.svg");
const ucIcon = loadInlineSvgDataUrl("browsers/uc.svg");
const yandexIcon = loadInlineSvgDataUrl("browsers/yandex.svg");
const androidIcon = loadInlineSvgDataUrl("os/android.svg");
const iosIcon = loadInlineSvgDataUrl("os/ios.svg");
const kaiosIcon = loadInlineSvgDataUrl("os/kaios.svg");
const linuxIcon = loadInlineSvgDataUrl("os/linux.svg");
const macosIcon = loadInlineSvgDataUrl("os/macos.svg");
const windowsIcon = loadInlineSvgDataUrl("os/windows.svg");
const desktopIcon = loadInlineSvgDataUrl("devices/desktop.svg");
const desktopTouchIcon = loadInlineSvgDataUrl("devices/desktop_touch.svg");
const mobileIcon = loadInlineSvgDataUrl("devices/mobile.svg");
const tabletIcon = loadInlineSvgDataUrl("devices/tablet.svg");
const discoveredViaIcon = loadInlineSvgDataUrl("journey/discovered-via.svg");
const eventIcon = loadInlineSvgDataUrl("journey/event.svg");
const goalIcon = loadInlineSvgDataUrl("journey/goal.svg");
const outboundLinkIcon = loadInlineSvgDataUrl("journey/outbound-link.svg");
const pageViewIcon = loadInlineSvgDataUrl("journey/page-view.svg");
const purchaseIcon = loadInlineSvgDataUrl("journey/purchase.svg");
const dashboardOverviewIcon = loadInlineSvgDataUrl("ui/dashboard-overview.svg");
const dashboardProjectsIcon = loadInlineSvgDataUrl("ui/dashboard-projects.svg");
const dashboardRevenueIcon = loadInlineSvgDataUrl("ui/dashboard-revenue.svg");
const dashboardRevenueSourcesIcon = loadInlineSvgDataUrl("ui/dashboard-revenue-sources.svg");
const dashboardCampaignsIcon = loadInlineSvgDataUrl("ui/dashboard-campaigns.svg");
const dashboardConversionsIcon = loadInlineSvgDataUrl("ui/dashboard-conversions.svg");
const dashboardSearchTermsIcon = loadInlineSvgDataUrl("ui/dashboard-search-terms.svg");
const dashboardAudienceIcon = loadInlineSvgDataUrl("ui/dashboard-audience.svg");
const dashboardPagesIcon = loadInlineSvgDataUrl("ui/dashboard-pages.svg");
const dashboardLandingPagesIcon = loadInlineSvgDataUrl("ui/dashboard-landing-pages.svg");
const dashboardVisitorsIcon = loadInlineSvgDataUrl("ui/dashboard-visitors.svg");
const dashboardCrawlersIcon = loadInlineSvgDataUrl("ui/dashboard-crawlers.svg");
const dashboardEventsIcon = loadInlineSvgDataUrl("ui/dashboard-events.svg");
const dashboardFunnelsIcon = loadInlineSvgDataUrl("ui/dashboard-funnels.svg");
const dashboardAskIcon = loadInlineSvgDataUrl("ui/dashboard-ask.svg");
const viewDetailsIcon = loadInlineSvgDataUrl("ui/view-details.svg");
const filterIcon = loadInlineSvgDataUrl("ui/filter.svg");
const highlightNegativeIcon = loadInlineSvgDataUrl("ui/highlight-negative.svg");

function inlineIconStyle(dataUrl: string, size: number, extra?: CSSProperties) {
  return style({
    flexShrink: 0,
    width: `${size}px`,
    height: `${size}px`,
    display: "inline-block",
    backgroundImage: `url("${dataUrl}")`,
    backgroundRepeat: "no-repeat",
    backgroundPosition: "center",
    backgroundSize: "contain",
    ...extra,
  });
}

function inlineMaskIconStyle(dataUrl: string, size: number, extra?: CSSProperties) {
  return style({
    flexShrink: 0,
    width: `${size}px`,
    height: `${size}px`,
    display: "inline-block",
    backgroundColor: "currentColor",
    maskImage: `url("${dataUrl}")`,
    maskRepeat: "no-repeat",
    maskPosition: "center",
    maskSize: "contain",
    WebkitMaskImage: `url("${dataUrl}")`,
    WebkitMaskRepeat: "no-repeat",
    WebkitMaskPosition: "center",
    WebkitMaskSize: "contain",
    ...extra,
  });
}

export const dashboardIconUnknown = inlineIconStyle(unknownIcon, 20);

export const dashboardIconBrowserBaidu = inlineIconStyle(baiduIcon, 20);
export const dashboardIconBrowserBrave = inlineIconStyle(braveIcon, 20);
export const dashboardIconBrowserChrome = inlineIconStyle(chromeIcon, 20);
export const dashboardIconBrowserDuckduckgo = inlineIconStyle(duckduckgoIcon, 20);
export const dashboardIconBrowserEdge = inlineIconStyle(edgeIcon, 20);
export const dashboardIconBrowserFirefox = inlineIconStyle(firefoxIcon, 20);
export const dashboardIconBrowserIe = inlineIconStyle(ieIcon, 20);
export const dashboardIconBrowserKai = inlineIconStyle(kaiIcon, 20);
export const dashboardIconBrowserOpera = inlineIconStyle(operaIcon, 20);
export const dashboardIconBrowserQq = inlineIconStyle(qqIcon, 20);
export const dashboardIconBrowserSafari = inlineIconStyle(safariIcon, 20);
export const dashboardIconBrowserSamsung = inlineIconStyle(samsungIcon, 20);
export const dashboardIconBrowserUc = inlineIconStyle(ucIcon, 20);
export const dashboardIconBrowserYandex = inlineIconStyle(yandexIcon, 20);

export const dashboardIconOsAndroid = inlineIconStyle(androidIcon, 20);
export const dashboardIconOsIos = inlineIconStyle(iosIcon, 20);
export const dashboardIconOsKaios = inlineIconStyle(kaiosIcon, 20);
export const dashboardIconOsLinux = inlineIconStyle(linuxIcon, 20);
export const dashboardIconOsMacos = inlineIconStyle(macosIcon, 20);
export const dashboardIconOsWindows = inlineIconStyle(windowsIcon, 20);

export const dashboardIconDeviceDesktop = inlineIconStyle(desktopIcon, 20);
export const dashboardIconDeviceDesktopTouch = inlineIconStyle(desktopTouchIcon, 20);
export const dashboardIconDeviceMobile = inlineIconStyle(mobileIcon, 20);
export const dashboardIconDeviceTablet = inlineIconStyle(tabletIcon, 20);

export const journeyIconDiscoveredVia = inlineIconStyle(discoveredViaIcon, 28);
export const journeyIconEvent = inlineIconStyle(eventIcon, 28);
export const journeyIconGoal = inlineIconStyle(goalIcon, 28);
export const journeyIconOutboundLink = inlineIconStyle(outboundLinkIcon, 28);
export const journeyIconPageView = inlineIconStyle(pageViewIcon, 28);
export const journeyIconPurchase = inlineIconStyle(purchaseIcon, 28);

export const gridEventIcon = inlineIconStyle(eventIcon, 18);
export const gridGoalIcon = inlineIconStyle(goalIcon, 18);
export const gridOutboundLinkIcon = inlineIconStyle(outboundLinkIcon, 18);

export const dashboardTabOverviewIcon = inlineIconStyle(dashboardOverviewIcon, 24);
export const dashboardTabProjectsIcon = inlineIconStyle(dashboardProjectsIcon, 24);
export const dashboardTabRevenueIcon = inlineIconStyle(dashboardRevenueIcon, 24);
/** Same revenue SVG as the tab icon, sized for hover-card highlight rows. */
export const hoverHighlightRevenueIcon = inlineIconStyle(dashboardRevenueIcon, 32);
/** Warning mark for negative highlights (highest exit/bounce, funnel drop-off). */
export const hoverHighlightNegativeIcon = inlineMaskIconStyle(highlightNegativeIcon, 16, {
  color: "#b91c1c",
  flex: "0 0 auto",
  marginTop: "2px",
});
export const gridWorstIcon = inlineMaskIconStyle(highlightNegativeIcon, 18, {
  color: "#b91c1c",
  flex: "0 0 auto",
});
export const dashboardTabRevenueSourcesIcon = inlineIconStyle(dashboardRevenueSourcesIcon, 24);
export const dashboardTabCampaignsIcon = inlineIconStyle(dashboardCampaignsIcon, 24);
export const dashboardTabConversionsIcon = inlineIconStyle(dashboardConversionsIcon, 24);
export const dashboardTabSearchTermsIcon = inlineIconStyle(dashboardSearchTermsIcon, 24);
export const dashboardTabAudienceIcon = inlineIconStyle(dashboardAudienceIcon, 24);
export const dashboardTabPagesIcon = inlineIconStyle(dashboardPagesIcon, 24);
export const dashboardTabLandingPagesIcon = inlineIconStyle(dashboardLandingPagesIcon, 24);
export const dashboardTabVisitorsIcon = inlineIconStyle(dashboardVisitorsIcon, 24);
export const dashboardTabCrawlersIcon = inlineIconStyle(dashboardCrawlersIcon, 24);
export const dashboardTabEventsIcon = inlineIconStyle(dashboardEventsIcon, 24);
export const dashboardTabFunnelsIcon = inlineIconStyle(dashboardFunnelsIcon, 24);
export const dashboardTabAskIcon = inlineIconStyle(dashboardAskIcon, 24);

export const gridInlineActionIcon = inlineIconStyle(viewDetailsIcon, 14, {
  flex: "0 0 auto",
});

export const gridFilterIcon = inlineMaskIconStyle(filterIcon, 14, {
  flex: "0 0 auto",
});
