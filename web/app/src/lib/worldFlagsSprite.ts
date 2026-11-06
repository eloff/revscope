export const WORLD_FLAGS_SPRITE_STYLESHEET =
  "https://unpkg.com/world-flags-sprite@0.0.2/stylesheets/flags32.css";
const WORLD_FLAGS_SPRITE_LINK_SELECTOR = 'link[data-world-flags-sprite="32"]';

/** Load world-flags-sprite CSS once per document (idempotent). */
export function ensureWorldFlagsSpriteStylesheet(): void {
  if (typeof document === "undefined") {
    return;
  }
  if (document.querySelector(WORLD_FLAGS_SPRITE_LINK_SELECTOR)) {
    return;
  }
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.type = "text/css";
  link.href = WORLD_FLAGS_SPRITE_STYLESHEET;
  link.setAttribute("data-world-flags-sprite", "32");
  document.head.appendChild(link);
}
