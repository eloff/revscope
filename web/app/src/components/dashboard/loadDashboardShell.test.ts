import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { loadDashboardShell } from "./loadDashboardShell";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

describe("loadDashboardShell", () => {
  it("shares one download between every caller", async () => {
    const first = loadDashboardShell();
    expect(loadDashboardShell()).toBe(first);
    expect((await first).default).toBeTypeOf("function");
    expect(loadDashboardShell()).toBe(first);
  }, 30_000);

  /**
   * Relative modules `file` imports or re-exports for their values; type-only
   * imports and dynamic `import()` don't bundle the target into the importer's chunk.
   */
  function staticImports(file: string): string[] {
    const source = readFileSync(file, "utf8");
    const specifiers = [
      ...source.matchAll(/^(?:import|export)\s+(?!type\s)[^;]*?from\s+"(\.[^"]+)"/gms),
      ...source.matchAll(/^import\s+"(\.[^"]+)"/gm),
    ].map((match) => match[1] as string);
    return specifiers.map((specifier) => resolve(dirname(file), specifier));
  }

  function resolveModule(path: string): string | undefined {
    for (const candidate of [
      path,
      `${path}.ts`,
      `${path}.tsx`,
      `${path}/index.ts`,
      `${path}/index.tsx`,
    ]) {
      try {
        readFileSync(candidate);
        return candidate;
      } catch {}
    }
    return undefined;
  }

  it("is the only way the home page embed reaches the dashboard", () => {
    const entry = resolve(__dirname, "DemoOverview.tsx");
    const seen = new Set<string>();
    const pending = [entry];
    while (pending.length > 0) {
      const file = pending.pop() as string;
      if (seen.has(file)) {
        continue;
      }
      seen.add(file);
      for (const target of staticImports(file)) {
        const module = resolveModule(target);
        if (module && /\.tsx?$/.test(module)) {
          pending.push(module);
        }
      }
    }
    const reached = [...seen].map((file) => file.slice(file.lastIndexOf("/") + 1));
    expect(reached).toContain("demoEmbedTabs.ts");
    expect(reached).not.toContain("DashboardShellContent.tsx");
    expect(reached).not.toContain("DashboardShell.tsx");
  });
});
