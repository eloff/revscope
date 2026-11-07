import { fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { signOut } from "../../../shared/test/auth";
import { clearCookie } from "../../../shared/test/cookies";
import { testSession } from "../../../shared/test/fixtures";
import { stubLocation } from "../../../shared/test/location";
import { fakeApple } from "../../../shared/test/signInScripts";
import SignupForm from "./SignupForm";

const SIGNUP = "/v2/public/auth/signup";

afterEach(async () => {
  clearCookie("revscope_last_login");
  vi.unstubAllGlobals();
  await signOut();
});

async function fillIn(name: string, email: string, password: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Website"), "https://www.Example.com/");
  await user.type(screen.getByLabelText("Name"), name);
  await user.clear(screen.getByLabelText("Email"));
  await user.type(screen.getByLabelText("Email"), email);
  await user.type(screen.getByLabelText("Password"), password);
  return user;
}

/** Dispatches submit directly: requestSubmit would stop at native constraint validation. */
function submit() {
  const form = screen.getByRole("button", { name: "Sign up" }).closest("form");
  if (form) fireEvent.submit(form);
}

describe("SignupForm", () => {
  it("prefills the email from the link", () => {
    stubLocation("/signup", "?email=ada%40example.com");
    render(() => <SignupForm />);
    expect(screen.getByLabelText("Email")).toHaveValue("ada@example.com");
  });

  it("creates the account, saves the session and opens the dashboard", async () => {
    const assign = stubLocation("/signup");
    const calls = mockApi("post", SIGNUP, testSession());
    render(() => <SignupForm />);
    await fillIn(" Ada ", "ada@example.com", "correctHorse9!");
    submit();

    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(calls[0]?.credentials).toBe("include");
    expect(calls[0]?.body).toEqual({
      name: "Ada",
      email: "ada@example.com",
      password: "correctHorse9!",
      account_name: "www.Example.com",
      project_name: "wwwexamplecom",
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    });
    expect(localStorage.getItem("revscope.auth.session")).toContain("ada@example.com");
    expect(document.cookie).toContain("revscope_last_login=email");
  });

  it("signs up with Apple, passing on the name Apple gave the browser", async () => {
    const assign = stubLocation("/signup");
    mockApi("get", "/v2/public/auth/providers", { google: null, apple: "co.revscope.web" });
    const calls = mockApi("post", "/v2/public/auth/id-token", testSession());
    fakeApple({
      authorization: { id_token: "apple-token" },
      user: { name: { firstName: "Ada", lastName: "Lovelace" } },
    });
    render(() => <SignupForm />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Project name"), "my_site");
    const button = await screen.findByRole("button", { name: "Continue with Apple" });
    await vi.waitFor(() => expect(button).toBeEnabled());
    await user.click(button);

    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(calls[0]?.body).toEqual({
      provider: "apple",
      id_token: "apple-token",
      name: "Ada Lovelace",
      signup: {
        account_name: "",
        project_name: "my_site",
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      },
    });
  });

  it.each([
    ["A", "ada@example.com", "correctHorse9!", "Name must be at least 2 characters."],
    ["Ada", "ada@", "correctHorse9!", "Enter a valid email address."],
    ["Ada", "ada@example.com", "short", "Password must be at least 10 characters."],
    [
      "Ada",
      "ada@example.com",
      "correct Horse9!",
      "Password cannot contain spaces or non-printable characters.",
    ],
  ])("validates %s / %s before calling the server", async (name, email, password, message) => {
    stubLocation("/signup");
    const calls = mockApi("post", SIGNUP, testSession());
    render(() => <SignupForm />);
    await fillIn(name, email, password);
    submit();
    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("shows the server's error, or a generic one for a malformed session", async () => {
    const assign = stubLocation("/signup");
    mockApi("post", SIGNUP, { error: "Signup is closed." }, 403);
    render(() => <SignupForm />);
    await fillIn("Ada", "ada@example.com", "correctHorse9!");
    submit();
    expect(await screen.findByText("Signup is closed.")).toBeInTheDocument();

    mockApi("post", SIGNUP, { ok: true });
    submit();
    expect(await screen.findByText("Unable to sign up.")).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
    expect(document.cookie).not.toContain("revscope_last_login");
  });

  it("names the project after the website until it's edited", async () => {
    stubLocation("/signup");
    render(() => <SignupForm />);
    const user = userEvent.setup();
    const website = screen.getByLabelText("Website");
    const projectName = screen.getByLabelText("Project name");
    await user.type(website, "shop.example.com");
    expect(projectName).toHaveValue("shopexamplecom");
    expect(screen.getByText(/ingest URL/)).toBeInTheDocument();

    await user.clear(projectName);
    await user.type(projectName, "My-Shop");
    expect(projectName).toHaveValue("myshop");
    await user.type(website, ".uk");
    expect(projectName).toHaveValue("myshop");
  });

  it("keeps following the website after a key the name drops", async () => {
    stubLocation("/signup");
    render(() => <SignupForm />);
    const user = userEvent.setup();
    const website = screen.getByLabelText("Website");
    const projectName = screen.getByLabelText<HTMLInputElement>("Project name");
    await user.type(website, "shop.example");
    await user.type(projectName, "-");
    await user.type(website, ".com");
    expect(projectName).toHaveValue("shopexamplecom");
  });

  it("keeps the caret where it was when a typed character is dropped", async () => {
    stubLocation("/signup");
    render(() => <SignupForm />);
    const projectName = screen.getByLabelText<HTMLInputElement>("Project name");
    await userEvent.setup().type(projectName, "myshop");
    // As if "-" had just been typed after "my".
    projectName.value = "my-shop";
    projectName.setSelectionRange(3, 3);
    fireEvent.input(projectName);
    expect(projectName).toHaveValue("myshop");
    expect(projectName.selectionStart).toBe(2);
  });

  it("leaves the name alone while a keyboard composes, then cleans it up", () => {
    stubLocation("/signup");
    render(() => <SignupForm />);
    const projectName = screen.getByLabelText<HTMLInputElement>("Project name");
    projectName.value = "My";
    projectName.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true }));
    expect(projectName).toHaveValue("My");
    fireEvent.compositionEnd(projectName);
    expect(projectName).toHaveValue("my");
  });

  it("checks the project name before Google or Apple signs up", async () => {
    stubLocation("/signup");
    mockApi("get", "/v2/public/auth/providers", { google: null, apple: "co.revscope.web" });
    const calls = mockApi("post", "/v2/public/auth/id-token", testSession());
    fakeApple({ authorization: { id_token: "apple-token" } });
    render(() => <SignupForm />);
    const button = await screen.findByRole("button", { name: "Continue with Apple" });
    await vi.waitFor(() => expect(button).toBeEnabled());
    await userEvent.setup().click(button);
    expect(await screen.findByText(/Project name must be at least 3/)).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("opens the login dialog for existing users", async () => {
    stubLocation("/signup");
    const opened = vi.fn();
    window.addEventListener("revscope:open-login", opened);
    render(() => <SignupForm />);
    await userEvent.click(screen.getByRole("button", { name: "Log in" }));
    expect(opened).toHaveBeenCalled();
    window.removeEventListener("revscope:open-login", opened);
  });
});
