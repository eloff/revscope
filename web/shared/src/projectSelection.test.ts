import { describe, expect, it } from "vitest";
import { testProject } from "../test/fixtures";
import {
  isDemoProjectId,
  planAllowsMultipleProjects,
  projectOptions,
  projectTimeZone,
  resolveProjectId,
} from "./projectSelection";

const options = projectOptions({
  zeta: testProject("3", "Europe/Paris"),
  alpha: testProject("9", ""),
  mid: testProject("5", "Asia/Tokyo"),
});

describe("projectOptions", () => {
  it("sorts by name and falls back to UTC for a blank zone", () => {
    expect(options).toEqual([
      { label: "alpha", value: "9", timezone: "UTC" },
      { label: "mid", value: "5", timezone: "Asia/Tokyo" },
      { label: "zeta", value: "3", timezone: "Europe/Paris" },
    ]);
  });
});

describe("projectTimeZone", () => {
  it("returns the selected project's zone", () => {
    expect(projectTimeZone(options, "3")).toBe("Europe/Paris");
  });

  it("uses UTC while options are loading or the id is unknown", () => {
    expect(projectTimeZone(undefined, "3")).toBe("UTC");
    expect(projectTimeZone(options, "404")).toBe("UTC");
  });
});

describe("resolveProjectId", () => {
  it("keeps a preferred project that is on the account", () => {
    expect(resolveProjectId(options, "5")).toBe("5");
  });

  it("falls back to the first project, or empty with none", () => {
    expect(resolveProjectId(options, "404")).toBe("9");
    expect(resolveProjectId([], "5")).toBe("");
  });
});

describe("isDemoProjectId", () => {
  it("matches only the demo project", () => {
    expect(isDemoProjectId("1")).toBe(true);
    expect(isDemoProjectId("10")).toBe(false);
  });
});

describe("planAllowsMultipleProjects", () => {
  it("allows more than one project from Growth up", () => {
    for (const plan of ["growth", "founder", "founder_friends", "enterprise"]) {
      expect(planAllowsMultipleProjects(plan)).toBe(true);
    }
    for (const plan of ["free", "starter", ""]) {
      expect(planAllowsMultipleProjects(plan)).toBe(false);
    }
  });
});
