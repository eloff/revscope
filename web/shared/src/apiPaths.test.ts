import { describe, expect, it } from "vitest";
import { testSession } from "../test/fixtures";
import { getApiBaseUrl } from "./apiBaseUrl";
import { projectApiPath, publicApiPath, sessionApiPath } from "./apiPaths";
import { authRequestInit } from "./apiRequest";
import { appPath, docsUrl, SELF_HOSTED, signupPath } from "./links";

describe("api paths", () => {
  it("builds session and public routes", () => {
    expect(sessionApiPath("me")).toBe("/v2/session/me");
    expect(publicApiPath("auth/login")).toBe("/v2/public/auth/login");
  });

  it("encodes the project id and omits an empty resource", () => {
    expect(projectApiPath("read", "a/b", "overview")).toBe("/v2/read/projects/a%2Fb/overview");
    expect(projectApiPath("session", "7", "")).toBe("/v2/session/projects/7");
  });
});

describe("getApiBaseUrl", () => {
  it("adds a scheme to a bare host", () => {
    expect(getApiBaseUrl()).toBe("http://localhost:4320");
  });
});

describe("authRequestInit", () => {
  it("sends the session cookie only for a signed-in user", () => {
    expect(authRequestInit(testSession(), { method: "POST" })).toEqual({
      method: "POST",
      credentials: "include",
    });
    expect(authRequestInit(undefined)).toEqual({ credentials: "omit" });
  });
});

describe("links on revscope.co", () => {
  it("is not the self-hosted build", () => {
    expect(SELF_HOSTED).toBe(false);
  });

  it("keeps app, docs and signup paths on this site", () => {
    expect(appPath("/dashboard")).toBe("/dashboard");
    expect(docsUrl("/docs/setup")).toBe("/docs/setup");
    expect(signupPath()).toBe("/signup");
  });
});
