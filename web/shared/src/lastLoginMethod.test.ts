import { afterEach, describe, expect, it } from "vitest";
import { clearCookie, setCookie } from "../test/cookies";
import { readLastLoginMethod } from "./lastLoginMethod";

afterEach(() => {
  clearCookie("revscope_last_login_method");
});

describe("readLastLoginMethod", () => {
  it("is empty without the cookie, and answers each method the API can set", () => {
    expect(readLastLoginMethod()).toBeUndefined();
    for (const method of ["google", "apple", "email"]) {
      setCookie(`revscope_last_login_method=${method}; Path=/`);
      expect(readLastLoginMethod()).toBe(method);
    }
  });

  it("ignores values it doesn't know and cookies with a similar name", () => {
    setCookie("revscope_last_login_method=facebook; Path=/");
    expect(readLastLoginMethod()).toBeUndefined();
    setCookie("revscope_last_login_method=constructor; Path=/");
    expect(readLastLoginMethod()).toBeUndefined();
    clearCookie("revscope_last_login_method");
    setCookie("other_revscope_last_login_method=google; Path=/");
    expect(readLastLoginMethod()).toBeUndefined();
    clearCookie("other_revscope_last_login_method");
  });
});
