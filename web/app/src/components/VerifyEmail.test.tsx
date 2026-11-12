import { render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../shared/test/api";
import { stubLocation } from "../../../shared/test/location";
import VerifyEmail from "./VerifyEmail";

const VERIFY = "/v2/public/auth/verify-email";

afterEach(() => vi.unstubAllGlobals());

describe("VerifyEmail", () => {
  it("confirms the email as soon as the link opens", async () => {
    stubLocation("/verify-email", "?token=%20abc%20");
    const calls = mockApi("post", VERIFY, { ok: true });
    // The page refreshes the signed-in tab's session afterwards.
    mockApi("get", "/v2/session/me", {}, 401);
    render(() => <VerifyEmail />);
    expect(await screen.findByText(/Your email is confirmed/)).toBeInTheDocument();
    expect(calls[0]?.body).toEqual({ token: "abc" });
    expect(screen.getByRole("link", { name: "Go to your dashboard" })).toHaveAttribute(
      "href",
      "/dashboard",
    );
  });

  it("shows why a link didn't work", async () => {
    stubLocation("/verify-email", "?token=abc");
    mockApi("post", VERIFY, { error: "This confirmation link is invalid or has expired." }, 400);
    render(() => <VerifyEmail />);
    expect(
      await screen.findByText("This confirmation link is invalid or has expired."),
    ).toBeInTheDocument();
  });

  it("reports a link without a token, and calls nothing", () => {
    stubLocation("/verify-email");
    const calls = mockApi("post", VERIFY, { ok: true });
    render(() => <VerifyEmail />);
    expect(screen.getByText("This confirmation link is missing its token.")).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });
});
