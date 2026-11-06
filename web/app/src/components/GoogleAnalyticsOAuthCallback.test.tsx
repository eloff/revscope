import { render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { signIn, signOut } from "../../../shared/test/auth";
import { testProject, testSession } from "../../../shared/test/fixtures";
import { stubLocation } from "../../../shared/test/location";
import { takePendingConnection } from "../lib/gaImportApi";
import GoogleAnalyticsOAuthCallback from "./GoogleAnalyticsOAuthCallback";

const session = testSession({ projects: { acme_test: testProject("7", "UTC") } });

beforeEach(() => {
  sessionStorage.clear();
});

afterEach(async () => {
  await signOut();
});

const PATH = "/v2/session/projects/7/ga-imports/connect";

describe("GoogleAnalyticsOAuthCallback", () => {
  it("finishes the connection on the state's project and returns to Settings", async () => {
    stubLocation("/oauth/google-analytics", "?code=abc&state=7.token");
    signIn(session);
    const connection = {
      connection_id: "conn",
      properties: [{ id: "123", name: "Acme", account: "Acme" }],
      known_hostnames: [],
    };
    const connects = mockApi("post", PATH, connection);
    render(() => <GoogleAnalyticsOAuthCallback />);
    await waitFor(() => expect(window.location.replace).toHaveBeenCalled());
    expect(connects[0].body).toEqual({ oauth_code: "abc", oauth_state: "7.token" });
    expect(takePendingConnection("7")).toEqual(connection);
  });

  it("explains a refused consent instead of calling the API", async () => {
    stubLocation("/oauth/google-analytics", "?error=access_denied&state=7.token");
    signIn(session);
    const connects = mockApi("post", PATH, {});
    render(() => <GoogleAnalyticsOAuthCallback />);
    expect(await screen.findByText(/didn't grant access/)).toBeTruthy();
    expect(connects).toHaveLength(0);
  });

  it("explains an expired sign-in", async () => {
    stubLocation("/oauth/google-analytics", "?code=abc&state=7.token");
    signIn(session);
    mockApi(
      "post",
      PATH,
      { error: "this Google sign-in has expired; start again from Settings" },
      400,
    );
    render(() => <GoogleAnalyticsOAuthCallback />);
    expect(await screen.findByText(/has expired/)).toBeTruthy();
  });

  it("asks a logged out visitor to log in", async () => {
    stubLocation("/oauth/google-analytics", "?code=abc&state=7.token");
    render(() => <GoogleAnalyticsOAuthCallback />);
    expect(await screen.findByText(/Log in, then connect/)).toBeTruthy();
  });
});
