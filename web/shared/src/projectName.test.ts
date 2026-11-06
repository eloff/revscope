import { describe, expect, it } from "vitest";
import {
  isValidProjectName,
  normalizeWebsite,
  projectNameFromWebsite,
  sanitizeProjectName,
  validateProjectName,
} from "./projectName";

describe("normalizeWebsite", () => {
  it.each([
    ["https://example.com/pricing?x=1", "example.com"],
    ["  http://shop.example.com  ", "shop.example.com"],
    ["//cdn.example.com/a", "cdn.example.com"],
    ["example.com#top", "example.com"],
    ["example.com?ref=1", "example.com"],
    ["example.com", "example.com"],
    ["custom+scheme://host.io/x", "host.io"],
    ["1http://example.com", "1http:"],
  ])("normalizes %j to %j", (raw, expected) => {
    expect(normalizeWebsite(raw)).toBe(expected);
  });
});

describe("projectNameFromWebsite", () => {
  it("matches the name the server derives from a website", () => {
    expect(projectNameFromWebsite("https://Shop.Example.com/pricing?x=1")).toBe("shopexamplecom");
    expect(projectNameFromWebsite("//cdn.example.com/a")).toBe("cdnexamplecom");
    expect(projectNameFromWebsite(" my_blog ")).toBe("my_blog");
    expect(projectNameFromWebsite("My-Site.Example_1.COM")).toBe("mysiteexample_1com");
    expect(projectNameFromWebsite("bücher.de")).toBe("bcherde");
    expect(projectNameFromWebsite("1http://example.com")).toBe("1http");
    expect(projectNameFromWebsite("https://")).toBe("");
  });
});

describe("sanitizeProjectName", () => {
  it("lowercases and drops what a project name can't hold", () => {
    expect(sanitizeProjectName("My-Shop_2!")).toBe("myshop_2");
  });
});

describe("validateProjectName", () => {
  it("accepts names of at least 3 of [a-z0-9_]", () => {
    expect(isValidProjectName("abc")).toBe(true);
    expect(validateProjectName("my_shop")).toBeNull();
  });

  it("rejects short, malformed and reserved names", () => {
    expect(isValidProjectName("ab")).toBe(false);
    expect(validateProjectName("")).toMatch(/at least 3/);
    expect(validateProjectName("My-Shop")).toMatch(/lowercase letters/);
    expect(validateProjectName("demo")).toMatch(/reserved/);
  });
});
