import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../test/api";
import { testSession } from "../test/fixtures";
import { stubLocation } from "../test/location";
import { API, server } from "../test/server";
import { fakeGoogle } from "../test/signInScripts";
import { parseAuthSession } from "./authState";
import LoginDialog, { openLoginDialog } from "./LoginDialog";

const REMEMBERED_EMAIL_KEY = "revscope.auth.rememberedEmail";

async function renderDialog(props: { redirectTo?: string } = {}) {
  render(() => <LoginDialog {...props} />);
  return { user: userEvent.setup() };
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Log in" }));
  return screen.findByRole("dialog");
}

/** Dispatches submit directly: requestSubmit would stop at native constraint validation. */
function submit() {
  const form = screen.getByLabelText("Email").closest("form");
  if (form) fireEvent.submit(form);
}

let assign: ReturnType<typeof vi.fn>;

beforeEach(() => {
  assign = stubLocation("/");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("LoginDialog", () => {
  it("opens from the trigger with the login form", async () => {
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    expect(dialog).toHaveTextContent("Use your email and password");
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sign up" })).toHaveAttribute("href", "/signup");
  });

  it("opens when another island calls openLoginDialog", async () => {
    await renderDialog();
    openLoginDialog();
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("when controlled, has no trigger and leaves openLoginDialog to the header's dialog", async () => {
    const changes: boolean[] = [];
    const [open, setOpen] = createSignal(false);
    render(() => (
      <LoginDialog
        open={open()}
        onOpenChange={(next) => {
          changes.push(next);
          setOpen(next);
        }}
      />
    ));
    expect(screen.queryByRole("button", { name: "Log in" })).toBeNull();
    openLoginDialog();
    await Promise.resolve();
    expect(screen.queryByRole("dialog")).toBeNull();

    setOpen(true);
    const dialog = await screen.findByRole("dialog", { name: "Log in" });
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Close" }));
    expect(changes).toEqual([false]);
  });

  it("when controlled, reopens on the login form however it was left", async () => {
    const [open, setOpen] = createSignal(false);
    render(() => <LoginDialog open={open()} onOpenChange={setOpen} />);
    const user = userEvent.setup();
    setOpen(true);
    let dialog = await screen.findByRole("dialog", { name: "Log in" });
    await user.type(within(dialog).getByLabelText("Password"), "typed-secret");
    await user.click(within(dialog).getByRole("button", { name: "Forgot password?" }));
    expect(await screen.findByRole("dialog", { name: "Reset password" })).toBeInTheDocument();

    setOpen(false);
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    setOpen(true);
    dialog = await screen.findByRole("dialog", { name: "Log in" });
    expect(within(dialog).getByLabelText("Password")).toHaveValue("");
  });

  it("logs in, saves the session and goes to the dashboard", async () => {
    let body: unknown;
    let credentials: RequestCredentials | undefined;
    server.use(
      http.post(`${API}/v2/public/auth/login`, async ({ request }) => {
        body = await request.json();
        credentials = request.credentials;
        return HttpResponse.json(testSession());
      }),
    );
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "  ada@example.com ");
    await user.type(screen.getByLabelText("Password"), "correctHorse9!");
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));

    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(body).toEqual({ username: "ada@example.com", password: "correctHorse9!" });
    expect(credentials).toBe("include");
    const stored = JSON.parse(localStorage.getItem("revscope.auth.session") ?? "{}");
    expect(parseAuthSession(stored.user)).toEqual(testSession());
    expect(localStorage.getItem(REMEMBERED_EMAIL_KEY)).toBeNull();
  });

  it("remembers the email when asked and honours redirectTo", async () => {
    server.use(http.post(`${API}/v2/public/auth/login`, () => HttpResponse.json(testSession())));
    const { user } = await renderDialog({ redirectTo: "/oauth/authorize?x=1" });
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correctHorse9!");
    await user.click(screen.getByLabelText("Remember me"));
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));

    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/oauth/authorize?x=1"));
    expect(localStorage.getItem(REMEMBERED_EMAIL_KEY)).toBe("ada@example.com");
  });

  it("prefills a remembered email with remember me checked", async () => {
    localStorage.setItem(REMEMBERED_EMAIL_KEY, "ada@example.com");
    const { user } = await renderDialog();
    await openDialog(user);
    expect(screen.getByLabelText("Email")).toHaveValue("ada@example.com");
    expect(screen.getByLabelText("Remember me")).toBeChecked();
    expect(screen.getByRole("link", { name: "Sign up" })).toHaveAttribute(
      "href",
      "/signup?email=ada%40example.com",
    );
  });

  it("validates before calling the server", async () => {
    const { user } = await renderDialog();
    await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "short");
    submit();
    expect(await screen.findByText(/at least 10 characters/)).toBeInTheDocument();
  });

  it("lets a password with whitespace from before the rule reach the server", async () => {
    server.use(
      http.post(`${API}/v2/public/auth/login`, () =>
        HttpResponse.json({ error: "Invalid email or password." }, { status: 401 }),
      ),
    );
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correct Horse9!");
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));
    expect(await screen.findByText("Invalid email or password.")).toBeInTheDocument();
  });

  it("shows the server's error", async () => {
    server.use(
      http.post(`${API}/v2/public/auth/login`, () =>
        HttpResponse.json({ error: "Invalid email or password." }, { status: 401 }),
      ),
    );
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correctHorse9!");
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));
    expect(await screen.findByText("Invalid email or password.")).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });

  it("refuses a login response that isn't a session", async () => {
    server.use(http.post(`${API}/v2/public/auth/login`, () => HttpResponse.json({ ok: true })));
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correctHorse9!");
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));
    expect(await screen.findByText("Unable to log in.")).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });

  it("reports an unreachable server", async () => {
    server.use(http.post(`${API}/v2/public/auth/login`, () => HttpResponse.error()));
    const { user } = await renderDialog();
    const dialog = await openDialog(user);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correctHorse9!");
    await user.click(within(dialog).getByRole("button", { name: "Log in" }));
    expect(await screen.findByText("Unable to reach the server.")).toBeInTheDocument();
  });

  it("sends a reset link from forgot-password mode", async () => {
    let body: unknown;
    server.use(
      http.post(`${API}/v2/public/auth/forgot-password`, async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({});
      }),
    );
    const { user } = await renderDialog();
    await openDialog(user);
    await user.click(screen.getByRole("button", { name: "Forgot password?" }));
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.click(screen.getByRole("button", { name: "Send reset link" }));
    expect(await screen.findByText(/If an account exists for that email/)).toBeInTheDocument();
    expect(body).toEqual({ email: "ada@example.com" });

    await user.click(screen.getByRole("button", { name: "Back to log in" }));
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.queryByText(/If an account exists for that email/)).not.toBeInTheDocument();
  });

  it("shows a forgot-password error", async () => {
    server.use(
      http.post(`${API}/v2/public/auth/forgot-password`, () =>
        HttpResponse.json({ error: "Slow down." }, { status: 429 }),
      ),
    );
    const { user } = await renderDialog();
    await openDialog(user);
    await user.click(screen.getByRole("button", { name: "Forgot password?" }));
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.click(screen.getByRole("button", { name: "Send reset link" }));
    expect(await screen.findByText("Slow down.")).toBeInTheDocument();
  });

  it("logs in with Google and goes where it was asked to", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: "google-client", apple: null });
    const calls = mockApi("post", "/v2/public/auth/id-token", testSession());
    const google = fakeGoogle();
    const { user } = await renderDialog({ redirectTo: "/oauth/consent" });
    await openDialog(user);
    await vi.waitFor(() => expect(google.renderButton).toHaveBeenCalled());

    google.signIn("google-token");
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/oauth/consent"));
    expect(calls[0]?.credentials).toBe("include");
    expect(calls[0]?.body).toEqual({ provider: "google", id_token: "google-token" });
    expect(localStorage.getItem("revscope.auth.session")).toContain("ada@example.com");
  });

  it("says when a Google account has no RevScope account", async () => {
    mockApi("get", "/v2/public/auth/providers", { google: "google-client", apple: null });
    mockApi(
      "post",
      "/v2/public/auth/id-token",
      { error: "No RevScope account uses this Google account yet. Sign up first." },
      404,
    );
    const google = fakeGoogle();
    const { user } = await renderDialog();
    await openDialog(user);
    await vi.waitFor(() => expect(google.renderButton).toHaveBeenCalled());

    google.signIn("google-token");
    expect(
      await screen.findByText("No RevScope account uses this Google account yet. Sign up first."),
    ).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });
});
