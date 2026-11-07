import { describe, expect, it, vi } from "vitest";
import { createOnceLoader } from "./onceLoader";

describe("createOnceLoader", () => {
  it("shares one load between overlapping and later calls", async () => {
    let resolve!: (value: string) => void;
    const load = vi.fn(() => new Promise<string>((r) => (resolve = r)));
    const get = createOnceLoader(load);
    const first = get();
    const second = get();
    resolve("module");
    expect(await first).toBe("module");
    expect(await second).toBe("module");
    expect(await get()).toBe("module");
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("tries again after a failed load", async () => {
    const load = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error("chunk 404"))
      .mockResolvedValueOnce("module");
    const get = createOnceLoader(load);
    await expect(get()).rejects.toThrow("chunk 404");
    expect(await get()).toBe("module");
    expect(load).toHaveBeenCalledTimes(2);
  });
});
