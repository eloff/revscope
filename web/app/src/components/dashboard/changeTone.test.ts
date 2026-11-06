import { describe, expect, it } from "vitest";
import { changeToneForRatio } from "./changeTone";

describe("changeToneForRatio", () => {
  it("treats a rise as good unless lower is better", () => {
    expect(changeToneForRatio(0.1)).toBe("positive");
    expect(changeToneForRatio(-0.1)).toBe("negative");
    expect(changeToneForRatio(0.1, false)).toBe("negative");
    expect(changeToneForRatio(-0.1, false)).toBe("positive");
  });

  it("treats no change or no comparison as flat", () => {
    expect(changeToneForRatio(0)).toBe("neutral");
    expect(changeToneForRatio(0, false)).toBe("neutral");
    expect(changeToneForRatio(null)).toBe("neutral");
    expect(changeToneForRatio(Number.POSITIVE_INFINITY)).toBe("neutral");
  });
});
