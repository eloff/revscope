import { afterEach, describe, expect, it, vi } from "vitest";
import { clearCookie, setCookie } from "../test/cookies";
import { stubLocation } from "../test/location";
import { readLastLoginMethod, rememberLoginMethod } from "./lastLoginMethod";

afterEach(() => {
  clearCookie("revscope_last_login");
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("lastLoginMethod", () => {
  it("is empty until a method is remembered, then answers the latest", () => {
    expect(readLastLoginMethod()).toBeUndefined();
    rememberLoginMethod("google");
    expect(readLastLoginMethod()).toBe("google");
    rememberLoginMethod("email");
    expect(readLastLoginMethod()).toBe("email");
  });

  it("keeps the cookie a year across the whole host, Secure only over https", () => {
    const set = vi.spyOn(document, "cookie", "set");
    stubLocation("/");
    rememberLoginMethod("apple");
    expect(set).toHaveBeenLastCalledWith(
      "revscope_last_login=apple; Max-Age=31536000; Path=/; SameSite=Lax",
    );

    vi.stubGlobal("location", { protocol: "https:" });
    rememberLoginMethod("apple");
    expect(set).toHaveBeenLastCalledWith(
      "revscope_last_login=apple; Max-Age=31536000; Path=/; SameSite=Lax; Secure",
    );
  });

  it("ignores values it didn't write and cookies with a similar name", () => {
    setCookie("revscope_last_login=facebook; Path=/");
    expect(readLastLoginMethod()).toBeUndefined();
    clearCookie("revscope_last_login");
    setCookie("other_revscope_last_login=google; Path=/");
    expect(readLastLoginMethod()).toBeUndefined();
    clearCookie("other_revscope_last_login");
  });
});
