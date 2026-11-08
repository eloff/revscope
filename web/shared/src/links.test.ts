import { afterEach, describe, expect, it, vi } from "vitest";
import { stubLocation } from "../test/location";
import { returnPathFrom, signupHref } from "./links";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("signupHref", () => {
  it("carries the email and the page to return to", () => {
    expect(signupHref("")).toBe("/signup");
    expect(signupHref("ada@example.com", "/oauth/authorize?scope=cli")).toBe(
      "/signup?email=ada%40example.com&next=%2Foauth%2Fauthorize%3Fscope%3Dcli",
    );
  });
});

describe("returnPathFrom", () => {
  it("accepts a path on this site", () => {
    stubLocation("/signup");
    expect(returnPathFrom("?next=%2Foauth%2Fauthorize%3Fscope%3Dcli")).toBe(
      "/oauth/authorize?scope=cli",
    );
  });

  it("ignores a next it can't parse, rather than throwing after sign-in", () => {
    stubLocation("/signup");
    expect(returnPathFrom("?next=%2F%2F%25")).toBeNull();
    expect(returnPathFrom("?next=%2F%2F%5B")).toBeNull();
  });

  it("refuses anything that leaves the site", () => {
    stubLocation("/signup");
    expect(returnPathFrom("")).toBeNull();
    expect(returnPathFrom("?next=https%3A%2F%2Fevil.example%2F")).toBeNull();
    expect(returnPathFrom("?next=%2F%2Fevil.example%2F")).toBeNull();
    expect(returnPathFrom("?next=%2F%5Cevil.example%2F")).toBeNull();
    expect(returnPathFrom("?next=javascript%3Aalert(1)")).toBeNull();
    for (const next of [
      "/.//evil.example/",
      "/..//evil.example",
      "/%2e//evil.example",
      "/a/..//evil.example",
      "/.\\\\evil.example",
    ]) {
      expect(returnPathFrom(`?next=${encodeURIComponent(next)}`), next).toBeNull();
    }
  });
});
