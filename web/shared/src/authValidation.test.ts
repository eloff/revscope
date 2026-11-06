import { describe, expect, it } from "vitest";
import {
  EMAIL_PATTERN,
  validateAccountName,
  validateEmail,
  validateLoginPassword,
  validatePassword,
  validateUserName,
} from "./authValidation";

describe("validateAccountName", () => {
  it("accepts a domain", () => {
    expect(validateAccountName("example.com")).toBeNull();
  });

  it("rejects names under the minimum length", () => {
    expect(validateAccountName("a.io")).toMatch(/at least 5 characters/);
  });

  it("requires a dot-separated domain", () => {
    expect(validateAccountName("examplecom")).toMatch(/must include a domain/);
  });
});

describe("validateUserName", () => {
  it("requires two characters", () => {
    expect(validateUserName("A")).not.toBeNull();
    expect(validateUserName("Al")).toBeNull();
  });
});

describe("validateEmail", () => {
  it.each(["ada@example.com", "a.b+c@sub.example.co"])("accepts %s", (email) => {
    expect(validateEmail(email)).toBeNull();
  });

  it.each(["", "ada", "ada@example", "ada@example.c", "@example.com"])("rejects %j", (email) => {
    expect(validateEmail(email)).toBe("Enter a valid email address.");
  });

  it("compiles as an input pattern, which browsers parse with the v flag", () => {
    const inputPattern = new RegExp(`^(?:${EMAIL_PATTERN.source})$`, "v");
    expect(inputPattern.test("a.b_c%d+e-f@sub-domain.example.co")).toBe(true);
  });
});

describe("validatePassword", () => {
  it("accepts a varied password", () => {
    expect(validatePassword("correctHorse9!")).toBeNull();
  });

  it("enforces the minimum length", () => {
    expect(validatePassword("Ab1!")).toMatch(/at least 10 characters/);
  });

  it("requires two uppercase letters, digits or symbols", () => {
    expect(validatePassword("abcdefghiJ")).toMatch(/at least 2 uppercase/);
  });

  it("rejects all digits", () => {
    expect(validatePassword("1234567890")).toBe("Password cannot be all digits.");
  });

  it("rejects all uppercase letters", () => {
    expect(validatePassword("ABCDEFGHIJ")).toBe("Password cannot be all uppercase letters.");
  });

  it("requires half the characters to be unique", () => {
    expect(validatePassword("aaaaaaaaB1")).toMatch(/50% unique/);
  });

  // The same code points as FORBIDDEN_CODE_POINTS in auth.rs, plus a lone surrogate.
  it.each([
    0x20, 0x9, 0xa, 0xd, 0x85, 0xa0, 0x2028, 0x3000, 0x0, 0x8, 0x1b, 0x7f, 0x9f, 0xad, 0x200b,
    0x200c, 0x200d, 0x200e, 0x202e, 0x2060, 0xfeff, 0xe0001, 0x34f, 0xfe0f, 0x115f, 0x3164, 0xe000,
    0xf0000, 0xfdd0, 0x10ffff, 0xfff9, 0xd800,
  ])("rejects code point %d", (codePoint) => {
    expect(validatePassword(`correct${String.fromCodePoint(codePoint)}Horse9!`)).toBe(
      "Password cannot contain spaces or non-printable characters.",
    );
  });

  it.each([" correctHorse9!", "correctHorse9! "])("rejects the padded %j", (password) => {
    expect(validatePassword(password)).toMatch(/cannot contain spaces/);
  });

  it.each([
    "correctHorse9!~`@#$%^&*()_+-=[]{}|;:'\",.<>/?\\",
    "Contraseña-çafé-ß9",
    "Пароль-достаточной-9",
    "長いパスワードです長いパスワードA9",
    "correctHorse9!😀",
    "correctHorse9!\u0378",
  ])("accepts printable or unassigned %s", (password) => {
    expect(validatePassword(password)).toBeNull();
  });
});

describe("validateLoginPassword", () => {
  it.each(["correct Horse9!", "correct\0Horse9!", "correct\u200bHorse9!"])(
    "accepts %j, which could be chosen before the rule",
    (password) => {
      expect(validateLoginPassword(password)).toBeNull();
    },
  );

  it("still applies the other rules", () => {
    expect(validateLoginPassword("Ab1!")).toMatch(/at least 10 characters/);
  });
});
