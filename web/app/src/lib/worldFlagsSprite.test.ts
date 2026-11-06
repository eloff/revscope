import { afterEach, describe, expect, it } from "vitest";
import {
  ensureWorldFlagsSpriteStylesheet,
  WORLD_FLAGS_SPRITE_STYLESHEET,
} from "./worldFlagsSprite";

afterEach(() => {
  for (const link of document.head.querySelectorAll("link[data-world-flags-sprite]")) {
    link.remove();
  }
});

describe("ensureWorldFlagsSpriteStylesheet", () => {
  it("adds the flags stylesheet once per document", () => {
    ensureWorldFlagsSpriteStylesheet();
    ensureWorldFlagsSpriteStylesheet();
    const links = document.head.querySelectorAll<HTMLLinkElement>("link[data-world-flags-sprite]");
    expect(links).toHaveLength(1);
    expect(links[0]?.href).toBe(WORLD_FLAGS_SPRITE_STYLESHEET);
    expect(links[0]?.rel).toBe("stylesheet");
  });
});
