import { render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal, Show } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../test/api";
import { fakeApple, fakeGoogle } from "../test/signInScripts";
import {
  fetchSignInProviders,
  forgetSignInProviders,
  type IdTokenSignIn,
  knownSignInProviders,
} from "./idTokenSignIn";
import LoginDialog from "./LoginDialog";
import SocialSignIn from "./SocialSignIn";

afterEach(() => {
  vi.unstubAllGlobals();
});

function renderButtons() {
  const signIns: IdTokenSignIn[] = [];
  render(() => <SocialSignIn onSignIn={(signIn) => signIns.push(signIn)} />);
  return signIns;
}

describe("SocialSignIn", () => {
  it("renders nothing when the server has no provider on", async () => {
    const calls = mockApi("get", "/v2/public/auth/providers", { google: null, apple: null });
    renderButtons();
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(screen.queryByText("or use your email")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Continue with Apple" })).not.toBeInTheDocument();
  });

  it("draws Google's button for the server's client and hands on its tokens", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: "google-client", apple: null });
    const google = fakeGoogle();
    const signIns = renderButtons();
    await waitFor(() => expect(google.renderButton).toHaveBeenCalled());
    expect(screen.getByText("or use your email")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Continue with Apple" })).not.toBeInTheDocument();

    google.signIn("google-token");
    expect(signIns).toEqual([{ provider: "google", idToken: "google-token" }]);
  });

  it("asks Apple for the name and email and hands on the token with the name", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: null, apple: "co.revscope.web" });
    const apple = fakeApple({
      authorization: { id_token: "apple-token" },
      user: { name: { firstName: "Katherine", lastName: "Johnson" } },
    });
    const signIns = renderButtons();
    const button = await screen.findByRole("button", { name: "Continue with Apple" });
    await waitFor(() => expect(button).toBeEnabled());
    expect(apple.init).toHaveBeenCalledWith(
      expect.objectContaining({ clientId: "co.revscope.web", scope: "name email", usePopup: true }),
    );

    await userEvent.setup().click(button);
    await waitFor(() =>
      expect(signIns).toEqual([
        { provider: "apple", idToken: "apple-token", name: "Katherine Johnson" },
      ]),
    );
  });

  it("does nothing when the Apple popup is closed, and says when Apple fails", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: null, apple: "co.revscope.web" });
    fakeApple({ error: "popup_closed_by_user" });
    const signIns = renderButtons();
    const button = await screen.findByRole("button", { name: "Continue with Apple" });
    await waitFor(() => expect(button).toBeEnabled());
    const user = userEvent.setup();
    await user.click(button);
    expect(signIns).toEqual([]);
    expect(screen.queryByText("Apple sign-in didn't work. Try again.")).not.toBeInTheDocument();

    fakeApple({ error: "invalid_client" });
    await user.click(button);
    expect(await screen.findByText("Apple sign-in didn't work. Try again.")).toBeInTheDocument();
    expect(signIns).toEqual([]);
  });

  it("sends Google sign-ins to the newest buttons on the page, not a closed dialog's", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: "google-client", apple: null });
    const google = fakeGoogle();
    const page: IdTokenSignIn[] = [];
    const overlay: IdTokenSignIn[] = [];
    const [showOverlay, setShowOverlay] = createSignal(false);
    render(() => (
      <>
        <LoginDialog />
        <SocialSignIn onSignIn={(signIn) => page.push(signIn)} />
        <Show when={showOverlay()}>
          <SocialSignIn onSignIn={(signIn) => overlay.push(signIn)} />
        </Show>
      </>
    ));
    await waitFor(() => expect(google.renderButton).toHaveBeenCalledTimes(1));
    google.signIn("first");
    expect(page.map((signIn) => signIn.idToken)).toEqual(["first"]);

    setShowOverlay(true);
    await waitFor(() => expect(google.renderButton).toHaveBeenCalledTimes(2));
    google.signIn("second");
    expect(overlay.map((signIn) => signIn.idToken)).toEqual(["second"]);

    setShowOverlay(false);
    google.signIn("third");
    expect(page.map((signIn) => signIn.idToken)).toEqual(["first", "third"]);
  });

  it("shares one request with a form mounted mid-request, which gets Google's sign-ins", async () => {
    let release: () => void = () => {};
    const waits = new Promise<void>((resolve) => {
      release = resolve;
    });
    const calls = mockApi("get", "/v2/public/auth/providers", async () => {
      await waits;
      return Response.json({ google: "google-client", apple: null });
    });
    const google = fakeGoogle();
    const page: IdTokenSignIn[] = [];
    const overlay: IdTokenSignIn[] = [];
    const [showOverlay, setShowOverlay] = createSignal(false);
    render(() => (
      <>
        <SocialSignIn onSignIn={(signIn) => page.push(signIn)} />
        <Show when={showOverlay()}>
          <SocialSignIn onSignIn={(signIn) => overlay.push(signIn)} />
        </Show>
      </>
    ));
    await waitFor(() => expect(calls).toHaveLength(1));
    setShowOverlay(true);
    release();
    await waitFor(() => expect(google.renderButton).toHaveBeenCalledTimes(2));
    expect(calls).toHaveLength(1);

    google.signIn("token");
    expect(overlay.map((signIn) => signIn.idToken)).toEqual(["token"]);
    expect(page).toEqual([]);
  });

  it("asks the server once per page, and shows a later form's buttons at once", async () => {
    const calls = mockApi("get", "/v2/public/auth/providers", {
      google: null,
      apple: "co.revscope.web",
    });
    fakeApple({ authorization: { id_token: "apple-token" } });
    const [showSecond, setShowSecond] = createSignal(false);
    render(() => (
      <>
        <SocialSignIn onSignIn={() => {}} />
        <Show when={showSecond()}>
          <SocialSignIn onSignIn={() => {}} />
        </Show>
      </>
    ));
    await screen.findByRole("button", { name: "Continue with Apple" });
    setShowSecond(true);
    // Synchronously: no request, no wait for one.
    expect(screen.getAllByRole("button", { name: "Continue with Apple" })).toHaveLength(2);
    // Long enough for a second request to have reached the mock, had one been sent.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(calls).toHaveLength(1);
  });

  it("asks again after a failed request instead of hiding the buttons for good", async () => {
    let calls = 0;
    mockApi("get", "/v2/public/auth/providers", () => {
      calls += 1;
      return calls === 1
        ? Response.json({ error: "down" }, { status: 503 })
        : Response.json({ google: null, apple: "co.revscope.web" });
    });
    fakeApple({ authorization: { id_token: "apple-token" } });
    const [showSecond, setShowSecond] = createSignal(false);
    render(() => (
      <>
        <SocialSignIn onSignIn={() => {}} />
        <Show when={showSecond()}>
          <SocialSignIn onSignIn={() => {}} />
        </Show>
      </>
    ));
    // The same request the form made, so this waits out its failure.
    await fetchSignInProviders();
    expect(calls).toBe(1);
    expect(screen.queryByRole("button", { name: "Continue with Apple" })).not.toBeInTheDocument();
    // The form that got the failure stays without buttons; the next one asks again.
    setShowSecond(true);
    expect(await screen.findAllByRole("button", { name: "Continue with Apple" })).toHaveLength(1);
    expect(calls).toBe(2);
  });

  it("doesn't let a forgotten request's answer stand in for a newer one", async () => {
    let release: () => void = () => {};
    const waits = new Promise<void>((resolve) => {
      release = resolve;
    });
    let calls = 0;
    mockApi("get", "/v2/public/auth/providers", async () => {
      calls += 1;
      if (calls === 1) {
        await waits;
        return Response.json({ google: null, apple: null });
      }
      return Response.json({ google: "google-client", apple: null });
    });
    const stale = fetchSignInProviders();
    await waitFor(() => expect(calls).toBe(1));
    forgetSignInProviders();
    const fresh = fetchSignInProviders();
    release();
    await stale;
    expect(knownSignInProviders()).toBeUndefined();
    expect(await fresh).toEqual({ google: "google-client", apple: null });
    expect(knownSignInProviders()).toEqual({ google: "google-client", apple: null });
  });
});
