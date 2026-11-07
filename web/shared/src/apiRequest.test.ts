import { renderHook } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../test/api";
import { signIn, signOut } from "../test/auth";
import { readJson } from "./apiRequest";
import { revalidateAuthSession, useAuthState } from "./authState";

function response(body: string, status: number): Response {
  return new Response(body, { status, headers: { "Content-Type": "application/json" } });
}

describe("readJson", () => {
  it("answers the payload of a successful response", async () => {
    await expect(readJson(response('{"rows":[1]}', 200), "fallback")).resolves.toEqual({
      rows: [1],
    });
  });

  it("throws the server's error message for a failed response", async () => {
    await expect(
      readJson(response('{"error":"end must be after start"}', 400), "fallback"),
    ).rejects.toThrow("end must be after start");
  });

  it("throws the fallback when a failed response carries no message", async () => {
    await expect(readJson(response("{}", 500), "fallback")).rejects.toThrow("fallback");
  });

  it("throws the fallback rather than a SyntaxError for a proxy's HTML page", async () => {
    const html = new Response("<html><body>502 Bad Gateway</body></html>", { status: 502 });
    await expect(readJson(html, "fallback")).rejects.toThrow(new Error("fallback"));
  });

  it("throws the fallback when a successful response is not JSON", async () => {
    await expect(readJson(response("<html></html>", 200), "fallback")).rejects.toThrow("fallback");
  });
});

describe("readJson on a 404", () => {
  afterEach(async () => {
    await signOut();
  });

  it("re-reads the account when a project is gone, so its list drops it", async () => {
    const session = signIn();
    const remaining = { ...session, projects: {} };
    const me = mockApi("get", "/v2/session/me", remaining);
    const { result } = renderHook(useAuthState);
    await expect(
      readJson(response('{"error":"project not found","code":"not_found"}', 404), "fallback"),
    ).rejects.toThrow("project not found");
    await vi.waitFor(() => expect(result.authUser()).toEqual(remaining));
    expect(me).toHaveLength(1);
  });

  it("leaves the account alone for any other 404", async () => {
    const session = signIn();
    const { result } = renderHook(useAuthState);
    await expect(
      readJson(response('{"error":"conversation not found"}', 404), "fallback"),
    ).rejects.toThrow("conversation not found");
    expect(result.authUser()).toEqual(session);
  });
});

describe("readJson on a 401", () => {
  afterEach(async () => {
    await signOut();
  });

  const rejected = () => response('{"error":"invalid session"}', 401);

  it("signs the tab out when /me refuses the session too", async () => {
    signIn();
    const me = mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    const { result } = renderHook(useAuthState);
    await expect(readJson(rejected(), "fallback")).rejects.toThrow("invalid session");
    await vi.waitFor(() => expect(result.authUser()).toBeNull());
    expect(me).toHaveLength(1);
  });

  it("stays signed in when /me accepts the session", async () => {
    const session = signIn();
    const me = mockApi("get", "/v2/session/me", session);
    const { result } = renderHook(useAuthState);
    await expect(readJson(rejected(), "fallback")).rejects.toThrow("invalid session");
    await vi.waitFor(() => expect(me).toHaveLength(1));
    // Joins the re-check readJson started, so this waits for /me's answer.
    await revalidateAuthSession();
    expect(result.authUser()).toEqual(session);
    expect(me).toHaveLength(1);
  });

  it("leaves a signed-out tab alone", async () => {
    await expect(readJson(rejected(), "fallback")).rejects.toThrow("invalid session");
  });

  it("ignores a 403, which a live session gets for an action it isn't allowed", async () => {
    const session = signIn();
    const { result } = renderHook(useAuthState);
    await expect(readJson(response('{"error":"admin only"}', 403), "fallback")).rejects.toThrow(
      "admin only",
    );
    expect(result.authUser()).toEqual(session);
  });
});
