/** Project names are at least this many of `[a-z0-9_]`; shorter ones are reserved. */
export const MIN_PROJECT_NAME_LENGTH = 3;

/** Readable without signing in, so no account may take them (`PUBLIC_PROJECT_NAMES`). */
const RESERVED_PROJECT_NAMES = ["demo"];

function stripUrlScheme(value: string): string {
  if (value.startsWith("//")) {
    return value.slice(2);
  }
  const schemeEnd = value.indexOf("://");
  if (schemeEnd <= 0) {
    return value;
  }
  return /^[A-Za-z][A-Za-z0-9+.-]*$/.test(value.slice(0, schemeEnd))
    ? value.slice(schemeEnd + 3)
    : value;
}

/**
 * The host a website URL or bare host is stored as: no scheme, path, query or
 * fragment. Mirrors `normalize_website` in revscope/src/projects.rs.
 */
export function normalizeWebsite(raw: string): string {
  return stripUrlScheme(raw.trim()).split(/[/?#]/)[0] ?? "";
}

/** Lowercased, with everything outside `[a-z0-9_]` dropped. */
export function sanitizeProjectName(raw: string): string {
  return raw.replace(/[A-Z]/g, (letter) => letter.toLowerCase()).replace(/[^a-z0-9_]/g, "");
}

/**
 * The project name the server derives from a website or bare name, so a form
 * can show it before submitting. Mirrors `project_name_from_website` in
 * revscope/src/projects.rs.
 */
export function projectNameFromWebsite(raw: string): string {
  return sanitizeProjectName(normalizeWebsite(raw));
}

export function isValidProjectName(name: string): boolean {
  return new RegExp(`^[a-z0-9_]{${MIN_PROJECT_NAME_LENGTH},}$`).test(name);
}

/** The server's messages for a name a signup sends (`signup_project_name`). */
export function validateProjectName(name: string): string | null {
  if (!isValidProjectName(name)) {
    return `Project name must be at least ${MIN_PROJECT_NAME_LENGTH} lowercase letters, numbers or underscores.`;
  }
  if (RESERVED_PROJECT_NAMES.includes(name)) {
    return "This project name is reserved. Try a different name.";
  }
  return null;
}
