import { fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { stubLocation } from "../../../shared/test/location";
import ResetPasswordForm from "./ResetPasswordForm";

const RESET = "/v2/public/auth/reset-password";

afterEach(() => vi.unstubAllGlobals());

async function submitPassword(password: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/password/i), password);
  const form = screen.getByRole("button", { name: /password/i }).closest("form");
  // Dispatched directly: requestSubmit would stop at native constraint validation.
  if (form) fireEvent.submit(form);
  return user;
}

describe("ResetPasswordForm", () => {
  it("reports a link without a token", () => {
    stubLocation("/reset-password");
    render(() => <ResetPasswordForm />);
    expect(screen.getByText("Password reset link is missing a token.")).toBeInTheDocument();
  });

  it("resets the password and offers to log in", async () => {
    stubLocation("/reset-password", "?token=%20abc%20");
    const calls = mockApi("post", RESET, {});
    render(() => <ResetPasswordForm />);
    expect(screen.getByRole("heading")).toHaveTextContent("Choose a new password");

    await submitPassword("correctHorse9!");
    expect(await screen.findByText(/Password updated/)).toBeInTheDocument();
    expect(calls[0]?.body).toEqual({ token: "abc", password: "correctHorse9!" });
    expect(screen.getByLabelText("New password")).toHaveValue("");

    const opened = vi.fn();
    window.addEventListener("revscope:open-login", opened);
    await userEvent.click(screen.getByRole("button", { name: "Log in" }));
    expect(opened).toHaveBeenCalled();
  });

  it("words an invite as choosing a first password", async () => {
    stubLocation("/reset-password", "?token=abc&invite");
    mockApi("post", RESET, {});
    render(() => <ResetPasswordForm />);
    expect(screen.getByRole("heading")).toHaveTextContent("Choose your password");
    await submitPassword("correctHorse9!");
    expect(await screen.findByText("Password set. Log in to get started.")).toBeInTheDocument();
  });

  it("validates the password before calling the server", async () => {
    stubLocation("/reset-password", "?token=abc");
    const calls = mockApi("post", RESET, {});
    render(() => <ResetPasswordForm />);
    await submitPassword("1234567890");
    expect(await screen.findByText("Password cannot be all digits.")).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("rejects a new password with whitespace before calling the server", async () => {
    stubLocation("/reset-password", "?token=abc");
    const calls = mockApi("post", RESET, {});
    render(() => <ResetPasswordForm />);
    await submitPassword("correct Horse9!");
    expect(await screen.findByText(/cannot contain spaces/)).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("shows an expired-link error from the server", async () => {
    stubLocation("/reset-password", "?token=abc");
    mockApi("post", RESET, { error: "This link has expired." }, 400);
    render(() => <ResetPasswordForm />);
    await submitPassword("correctHorse9!");
    expect(await screen.findByText("This link has expired.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Log in" })).not.toBeInTheDocument();
  });

  it("reports an unreachable server", async () => {
    stubLocation("/reset-password", "?token=abc");
    mockApi("post", RESET, () => HttpResponse.error());
    render(() => <ResetPasswordForm />);
    await submitPassword("correctHorse9!");
    expect(await screen.findByText("Unable to reach the server.")).toBeInTheDocument();
  });
});
