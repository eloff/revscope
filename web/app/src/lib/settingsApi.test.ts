import { describe, expect, it } from "vitest";
import {
  finishedSyncError,
  INTEGRATION_META,
  INTEGRATION_TYPES,
  parseServiceAccountKey,
  timezoneOptions,
} from "./settingsApi";

const JANUARY = new Date("2026-01-15T12:00:00Z");
const JULY = new Date("2026-07-15T12:00:00Z");

describe("timezoneOptions", () => {
  it("labels zones with their current offset, west to east", () => {
    const options = timezoneOptions(undefined, JANUARY);
    expect(options[0]?.value).toBe("Pacific/Pago_Pago");
    expect(options.at(-1)?.value).toBe("Pacific/Kiritimati");
    expect(options.find((option) => option.value === "America/New_York")?.label).toBe(
      "(UTC-05:00) New York, Toronto",
    );
    expect(options.find((option) => option.value === "Asia/Kolkata")?.label).toBe(
      "(UTC+05:30) Mumbai, New Delhi",
    );
  });

  it("follows daylight saving time", () => {
    const july = timezoneOptions(undefined, JULY);
    expect(july.find((option) => option.value === "America/New_York")?.label).toBe(
      "(UTC-04:00) New York, Toronto",
    );
  });

  it("appends a saved zone that isn't curated", () => {
    const options = timezoneOptions("Europe/Lisbon", JANUARY);
    expect(options.find((option) => option.value === "Europe/Lisbon")?.label).toBe(
      "(UTC+00:00) Europe/Lisbon",
    );
    expect(timezoneOptions("UTC", JANUARY)).toHaveLength(
      timezoneOptions(undefined, JANUARY).length,
    );
  });

  it("sorts an unknown zone last with a bare label", () => {
    const options = timezoneOptions("Mars/Olympus_Mons", JANUARY);
    expect(options.at(-1)).toEqual({ value: "Mars/Olympus_Mons", label: "Mars/Olympus_Mons" });
  });
});

describe("INTEGRATION_META", () => {
  it("describes every integration type with at least one secret field", () => {
    for (const type of INTEGRATION_TYPES) {
      expect(INTEGRATION_META[type].type).toBe(type);
      expect(INTEGRATION_META[type].fields.some((field) => field.secret)).toBe(true);
    }
  });
});

describe("finishedSyncError", () => {
  it("shows the last failure while a new sync runs", () => {
    expect(finishedSyncError({ last_sync_error: null, last_finished_sync_error: "Revoked" })).toBe(
      "Revoked",
    );
  });

  it("falls back to the only error an older server sends", () => {
    expect(finishedSyncError({ last_sync_error: "Revoked" })).toBe("Revoked");
  });

  it("reports nothing after a success", () => {
    expect(finishedSyncError({ last_sync_error: null, last_finished_sync_error: null })).toBeNull();
  });
});

describe("parseServiceAccountKey", () => {
  it("takes the email and key from a key file, keeping the key on one line", () => {
    const file = JSON.stringify({
      type: "service_account",
      client_email: "revscope@example.iam.gserviceaccount.com",
      private_key: "-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\n",
    });
    expect(parseServiceAccountKey(file)).toEqual({
      clientEmail: "revscope@example.iam.gserviceaccount.com",
      privateKey: "-----BEGIN PRIVATE KEY-----\\nABC\\n-----END PRIVATE KEY-----",
    });
  });

  it("refuses text that isn't a key file", () => {
    expect(parseServiceAccountKey("{")).toBeNull();
    expect(parseServiceAccountKey("null")).toBeNull();
    expect(parseServiceAccountKey('{"client_email":"a@b"}')).toBeNull();
    expect(parseServiceAccountKey('{"client_email":"","private_key":"k"}')).toBeNull();
  });
});
