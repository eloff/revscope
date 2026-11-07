/** Set on the root element by EmbedLayout, for pages served to be framed on revscope.co. */
export const EMBED_PAGE_ATTRIBUTE = "data-revscope-embed";

/**
 * Whether this is an embed page (EmbedLayout). Keyed to the page rather than to
 * being framed: someone framing the dashboard itself, say in a self-hosted
 * dashboard panel, still gets its normal Back behaviour.
 */
export function isEmbedPage(): boolean {
  return document.documentElement.hasAttribute(EMBED_PAGE_ATTRIBUTE);
}

/**
 * Records a navigation in the URL. An embed shares the host page's session
 * history, where pushed entries would make the host's Back button step through
 * them before leaving, so embed navigations replace the current entry instead.
 */
export function writeHistoryUrl(url: URL) {
  if (isEmbedPage()) {
    window.history.replaceState(null, "", url);
  } else {
    window.history.pushState(null, "", url);
  }
}
