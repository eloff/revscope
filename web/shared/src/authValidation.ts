// biome-ignore lint/complexity/noUselessEscapeInRegex: browsers compile `<input pattern>` with the `v` flag, which requires `-` in a class to be escaped.
export const EMAIL_PATTERN = /^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$/;
const PASSWORD_COMPLEXITY_PATTERN = /[A-Z\d\W_]/g;
const ALL_DIGITS_PATTERN = /^\d+$/;
const ALL_UPPERCASE_PATTERN = /^[A-Z]+$/;
// FORBIDDEN_PASSWORD_CHARACTER in auth.rs explains this pattern. It adds only \p{Cs}, since lone
// surrogates can exist in a JavaScript string but not a Rust one.
const FORBIDDEN_PASSWORD_CHARACTER =
  /[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Z}\p{Default_Ignorable_Code_Point}\p{Noncharacter_Code_Point}]/u;

export const MIN_ACCOUNT_NAME_LENGTH = 5;
export const MIN_USER_NAME_LENGTH = 2;
export const MIN_PASSWORD_LENGTH = 10;
const MIN_PASSWORD_COMPLEXITY_COUNT = 2;
const MIN_PASSWORD_UNIQUE_RATIO = 0.5;

function passwordComplexityCount(password: string): number {
  return password.match(PASSWORD_COMPLEXITY_PATTERN)?.length ?? 0;
}

function passwordUniqueRatio(password: string): number {
  return new Set(password).size / password.length;
}

/** At least one character on either side of a `.` (e.g. mywebsite.com). */
export const WEBSITE_PATTERN = /^.+\..+$/;

export function validateAccountName(accountName: string): string | null {
  if (accountName.length < MIN_ACCOUNT_NAME_LENGTH) {
    return `Website must be at least ${MIN_ACCOUNT_NAME_LENGTH} characters.`;
  }
  if (!WEBSITE_PATTERN.test(accountName)) {
    return "Website must include a domain (e.g. mywebsite.com).";
  }
  return null;
}

export function validateUserName(name: string): string | null {
  if (name.length < MIN_USER_NAME_LENGTH) {
    return `Name must be at least ${MIN_USER_NAME_LENGTH} characters.`;
  }
  return null;
}

export function validateEmail(email: string): string | null {
  if (!EMAIL_PATTERN.test(email)) {
    return "Enter a valid email address.";
  }
  return null;
}

/** For choosing a password. Logging in uses validateLoginPassword. */
export function validatePassword(password: string): string | null {
  if (FORBIDDEN_PASSWORD_CHARACTER.test(password)) {
    return "Password cannot contain spaces or non-printable characters.";
  }
  return validateLoginPassword(password);
}

/** Omits the character rule, since accounts from before it can have passwords that break it. */
export function validateLoginPassword(password: string): string | null {
  if (password.length < MIN_PASSWORD_LENGTH) {
    return `Password must be at least ${MIN_PASSWORD_LENGTH} characters.`;
  }
  if (passwordComplexityCount(password) < MIN_PASSWORD_COMPLEXITY_COUNT) {
    return "Password must include at least 2 uppercase letters, digits, or symbols.";
  }
  if (ALL_DIGITS_PATTERN.test(password)) {
    return "Password cannot be all digits.";
  }
  if (ALL_UPPERCASE_PATTERN.test(password)) {
    return "Password cannot be all uppercase letters.";
  }
  if (passwordUniqueRatio(password) < MIN_PASSWORD_UNIQUE_RATIO) {
    return "Password must be at least 50% unique characters.";
  }
  return null;
}
