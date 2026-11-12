import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { mockApi } from "../../../../shared/test/api";
import { testSession } from "../../../../shared/test/fixtures";
import EmailVerificationBanner from "./EmailVerificationBanner";

const RESEND = "/v2/session/me/verify-email";

describe("EmailVerificationBanner", () => {
  it("asks for a confirmed email, and emails a link", async () => {
    const calls = mockApi("post", RESEND, { ok: true });
    render(() => <EmailVerificationBanner session={testSession({ account_confirmed: false })} />);
    expect(screen.getByRole("status").textContent).toContain("Confirm your email address");

    await userEvent.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(await screen.findByText(/Sending a link to ada@example\.com/)).toBeInTheDocument();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.credentials).toBe("include");
  });

  it("says why a new link wasn't sent", async () => {
    mockApi(
      "post",
      RESEND,
      { error: "too many attempts; please wait a few minutes and try again" },
      429,
    );
    render(() => <EmailVerificationBanner session={testSession({ account_confirmed: false })} />);
    await userEvent.click(screen.getByRole("button", { name: "Email me a link" }));
    expect(await screen.findByText(/too many attempts/)).toBeInTheDocument();
  });

  it("shows nothing once the email is confirmed", () => {
    render(() => <EmailVerificationBanner session={testSession()} />);
    expect(screen.queryByRole("status")).toBeNull();
  });
});
