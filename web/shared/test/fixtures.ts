import type { AuthProject, AuthSession } from "../src/authState";

export function testProject(id: string, timezone = "UTC"): AuthProject {
  return { id, settings: { timezone } };
}

export function testSession(overrides: Partial<AuthSession> = {}): AuthSession {
  return {
    name: "Ada Lovelace",
    email: "ada@example.com",
    account_id: "acct-1",
    user_id: "user-1",
    plan: "pro",
    is_admin: true,
    projects: { example_com: testProject("42", "America/New_York") },
    ...overrides,
  };
}
