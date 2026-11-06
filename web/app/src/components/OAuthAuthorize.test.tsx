import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { signIn, signOut } from "../../../shared/test/auth";
import { testProject, testSession } from "../../../shared/test/fixtures";
import { stubLocation } from "../../../shared/test/location";
import OAuthAuthorize from "./OAuthAuthorize";

const AUTHORIZE = "/v2/session/mcp/authorize";
const REQUEST =
  "?client_id=c1&redirect_uri=http%3A%2F%2F127.0.0.1%3A3333%2Fcb&state=s1&code_challenge=ch&code_challenge_method=S256";
const client = {
  client_id: "c1",
  client_name: "Claude Code",
  redirect_uri: "http://127.0.0.1:3333/cb",
};

afterEach(async () => {
  vi.unstubAllGlobals();
  await signOut();
});

describe("OAuthAuthorize", () => {
  it("explains a visit without an authorization request", async () => {
    stubLocation("/oauth/authorize");
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText(/Open it from your client/)).toBeInTheDocument();
  });

  it("asks a signed-out user to log in and come back here", async () => {
    stubLocation("/oauth/authorize", REQUEST);
    const calls = mockApi("get", AUTHORIZE, client);
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText(/Log in to connect/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Log in" })).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("asks to log in again, without a stale error, when the session has died", async () => {
    stubLocation("/oauth/authorize", REQUEST);
    signIn();
    mockApi("get", AUTHORIZE, { error: "invalid session" }, 401);
    mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText(/Log in to connect/)).toBeInTheDocument();
    expect(screen.queryByText("That authorization request is not valid.")).toBeNull();
  });

  it("asks to log in again when the session dies before the decision", async () => {
    stubLocation("/oauth/authorize", REQUEST);
    signIn();
    mockApi("get", AUTHORIZE, client);
    mockApi("post", AUTHORIZE, { error: "invalid session" }, 401);
    mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    render(() => <OAuthAuthorize />);
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText(/Log in to connect/)).toBeInTheDocument();
    expect(screen.queryByText("Unable to complete the request.")).toBeNull();
  });

  it("names the client, its redirect host and the projects it can read", async () => {
    stubLocation("/oauth/authorize", REQUEST);
    signIn(testSession({ projects: { zed_io: testProject("2"), acme_com: testProject("1") } }));
    const calls = mockApi("get", AUTHORIZE, client);
    render(() => <OAuthAuthorize />);
    expect(await screen.findByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:3333")).toBeInTheDocument();
    expect(screen.getByText("acme_com, zed_io")).toBeInTheDocument();
    expect(queryOf(calls[0])).toEqual({
      client_id: "c1",
      redirect_uri: "http://127.0.0.1:3333/cb",
    });
  });

  it("lets a member only deny, since connecting an agent is for admins", async () => {
    const assign = stubLocation("/oauth/authorize", REQUEST);
    signIn(testSession({ is_admin: false }));
    mockApi("get", AUTHORIZE, client);
    const posts = mockApi("post", AUTHORIZE, { redirect_url: "http://127.0.0.1:3333/cb?e=1" });
    render(() => <OAuthAuthorize />);
    expect(
      await screen.findByText(
        "Only account admins can connect an agent. Ask an admin to approve it.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Deny" }));
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("http://127.0.0.1:3333/cb?e=1"));
    expect(posts[0]?.body).toMatchObject({ approve: false });
  });

  it("lets any user approve the CLI, and says it acts as them", async () => {
    const assign = stubLocation("/oauth/authorize", `${REQUEST}&scope=cli`);
    signIn(testSession({ is_admin: false }));
    mockApi("get", AUTHORIZE, client);
    const posts = mockApi("post", AUTHORIZE, { redirect_url: "http://127.0.0.1:3333/cb?code=x" });
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText(/wants to act as you/)).toBeInTheDocument();
    expect(screen.queryByText(/Only account admins/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("http://127.0.0.1:3333/cb?code=x"));
    expect(posts[0]?.body).toMatchObject({ approve: true, scope: "cli" });
  });

  it("ignores an MCP client's own scope values", async () => {
    stubLocation("/oauth/authorize", `${REQUEST}&scope=read`);
    signIn(testSession({ is_admin: true }));
    mockApi("get", AUTHORIZE, client);
    const posts = mockApi("post", AUTHORIZE, { redirect_url: "http://127.0.0.1:3333/cb?code=x" });
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText(/wants read-only access/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    await vi.waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]?.body).not.toHaveProperty("scope");
  });

  it("rejects a redirect URI the client didn't register", async () => {
    stubLocation("/oauth/authorize", REQUEST);
    signIn();
    mockApi(
      "get",
      AUTHORIZE,
      { error: "invalid_request", error_description: "redirect_uri mismatch" },
      400,
    );
    render(() => <OAuthAuthorize />);
    expect(await screen.findByText("redirect_uri mismatch")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
  });

  it.each([
    ["Approve", true],
    ["Deny", false],
  ])("sends the decision for %s and follows the API's redirect", async (button, approve) => {
    const assign = stubLocation("/oauth/authorize", REQUEST);
    signIn();
    mockApi("get", AUTHORIZE, client);
    const posts = mockApi("post", AUTHORIZE, { redirect_url: "http://127.0.0.1:3333/cb?code=x" });
    render(() => <OAuthAuthorize />);
    await userEvent.click(await screen.findByRole("button", { name: button }));
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("http://127.0.0.1:3333/cb?code=x"));
    expect(posts[0]?.body).toEqual({
      client_id: "c1",
      redirect_uri: "http://127.0.0.1:3333/cb",
      state: "s1",
      code_challenge: "ch",
      code_challenge_method: "S256",
      approve,
    });
  });

  it("stays put when the API gives no redirect", async () => {
    const assign = stubLocation("/oauth/authorize", REQUEST);
    signIn();
    mockApi("get", AUTHORIZE, client);
    mockApi("post", AUTHORIZE, {});
    render(() => <OAuthAuthorize />);
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Unable to complete the request.")).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });
});
