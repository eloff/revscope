import { renderHook } from "@solidjs/testing-library";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { testProject, testSession } from "../test/fixtures";
import { API, server } from "../test/server";

const STORAGE_KEY = "revscope.auth.session";
const FIVE_MINUTES = 5 * 60 * 1000;
const NOW = new Date("2026-10-01T12:00:00Z").getTime();

// authState keeps the session in module-level signals, so each test loads a fresh copy.
async function loadAuthState() {
  vi.resetModules();
  return import("./authState");
}

function storeSession(fetchedAt: number, user: unknown = testSession()) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ user, fetched_at: fetchedAt }));
}

function storedSession() {
  const raw = localStorage.getItem(STORAGE_KEY);
  return raw ? JSON.parse(raw) : null;
}

/** Counts `/me` requests and answers each with `respond`. */
function mockMe(respond: () => Response) {
  const calls = { count: 0, credentials: [] as RequestCredentials[] };
  server.use(
    http.get(`${API}/v2/session/me`, ({ request }) => {
      calls.count += 1;
      calls.credentials.push(request.credentials);
      return respond();
    }),
  );
  return calls;
}

// Every fresh module copy adds its own `visibilitychange` listener; remove them
// so earlier tests' copies don't answer later tests' events.
let documentListeners: [string, EventListenerOrEventListenerObject][] = [];

beforeEach(() => {
  // `vi.waitFor` advances faked timers, so assertions compare against NOW rather than Date.now().
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  const addEventListener = document.addEventListener.bind(document);
  vi.spyOn(document, "addEventListener").mockImplementation((type, listener, options) => {
    documentListeners.push([type, listener]);
    addEventListener(type, listener, options);
  });
});

afterEach(() => {
  for (const [type, listener] of documentListeners) {
    document.removeEventListener(type, listener);
  }
  documentListeners = [];
  vi.restoreAllMocks();
  vi.useRealTimers();
  delete (window as unknown as Record<string, unknown>).dispatch;
});

describe("parseAuthSession", () => {
  it("accepts a complete payload and drops a non-string gravatar", async () => {
    const { parseAuthSession } = await loadAuthState();
    const session = testSession();
    expect(parseAuthSession({ ...session, gravatar_url: 5, extra: true })).toEqual(session);
  });

  it.each([
    ["null", null],
    ["a string", "user"],
    ["an empty name", { ...testSession(), name: "" }],
    ["an empty email", { ...testSession(), email: "" }],
    ["a missing account id", { ...testSession(), account_id: undefined }],
    ["a missing plan", { ...testSession(), plan: undefined }],
    ["a missing admin flag", { ...testSession(), is_admin: undefined }],
    ["an array of projects", { ...testSession(), projects: [] }],
    ["a project without an id", { ...testSession(), projects: { a: testProject("") } }],
    ["a project without a zone", { ...testSession(), projects: { a: testProject("1", "") } }],
  ])("rejects %s", async (_label, payload) => {
    const { parseAuthSession } = await loadAuthState();
    expect(parseAuthSession(payload)).toBeNull();
  });
});

describe("useAuthState", () => {
  it("resolves signed out without calling the API when nothing is stored", async () => {
    const calls = mockMe(() => HttpResponse.json(testSession()));
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    expect(result.authResolved()).toBe(true);
    expect(result.authUser()).toBeNull();
    expect(calls.count).toBe(0);
  });

  describe("self-hosted", () => {
    beforeEach(() => {
      vi.doMock("./links", async (importOriginal) => ({
        ...(await importOriginal<typeof import("./links")>()),
        SELF_HOSTED: true,
      }));
    });
    afterEach(() => {
      vi.doUnmock("./links");
    });

    it("asks the cookie when nothing is stored", async () => {
      const calls = mockMe(() => HttpResponse.json(testSession()));
      const { useAuthState } = await loadAuthState();
      const { result } = renderHook(useAuthState);
      expect(result.authResolved()).toBe(false);
      await vi.waitFor(() => expect(result.authResolved()).toBe(true));
      expect(result.authUser()).toEqual(testSession());
      expect(calls.credentials).toEqual(["include"]);
      expect(storedSession().user).toEqual(testSession());
    });

    it.each([
      ["a rejected cookie", () => new HttpResponse(null, { status: 401 })],
      ["a network failure", () => HttpResponse.error()],
    ])("resolves signed out on %s", async (_label, respond) => {
      mockMe(respond);
      const { useAuthState } = await loadAuthState();
      const { result } = renderHook(useAuthState);
      await vi.waitFor(() => expect(result.authResolved()).toBe(true));
      expect(result.authUser()).toBeNull();
    });
  });

  it("hydrates a fresh stored session without refetching", async () => {
    storeSession(Date.now() - 1000);
    const calls = mockMe(() => HttpResponse.json(testSession()));
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    expect(result.authUser()).toEqual(testSession());
    expect(calls.count).toBe(0);
  });

  it("drops an unreadable stored session", async () => {
    localStorage.setItem(STORAGE_KEY, "{not json");
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    expect(result.authUser()).toBeNull();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("drops a stored session that fails validation", async () => {
    storeSession(Date.now(), { name: "no email" });
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    expect(result.authUser()).toBeNull();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("refreshes a stale stored session with the cookie and keeps the update", async () => {
    storeSession(Date.now() - FIVE_MINUTES);
    const updated = testSession({ plan: "team" });
    const calls = mockMe(() => HttpResponse.json(updated));
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    await vi.waitFor(() => expect(result.authUser()?.plan).toBe("team"));
    expect(calls.credentials).toEqual(["include"]);
    expect(storedSession().user).toEqual(updated);
    expect(storedSession().fetched_at).toBeGreaterThanOrEqual(NOW);
  });

  it("shows a role change from a refresh in the open page", async () => {
    storeSession(Date.now() - FIVE_MINUTES);
    mockMe(() => HttpResponse.json(testSession({ is_admin: false })));
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    expect(result.authUser()?.is_admin).toBe(true);
    await vi.waitFor(() => expect(result.authUser()?.is_admin).toBe(false));
  });

  it("clears the session when the refresh is rejected", async () => {
    storeSession(0);
    mockMe(() => new HttpResponse(null, { status: 401 }));
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    await vi.waitFor(() => expect(result.authUser()).toBeNull());
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it.each([
    ["a server error", () => new HttpResponse(null, { status: 500 })],
    ["a network failure", () => HttpResponse.error()],
    ["an invalid payload", () => HttpResponse.json({ name: "x" })],
  ])("keeps the cached session on %s", async (_label, respond) => {
    storeSession(0);
    const calls = mockMe(respond);
    const { useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    await vi.waitFor(() => expect(calls.count).toBe(1));
    await Promise.resolve();
    expect(result.authUser()).toEqual(testSession());
    expect(storedSession()?.fetched_at).toBe(0);
  });

  it("shares one request between islands mounting together", async () => {
    storeSession(0);
    const calls = mockMe(() => HttpResponse.json(testSession()));
    const { useAuthState } = await loadAuthState();
    renderHook(useAuthState);
    renderHook(useAuthState);
    await vi.waitFor(() => expect(storedSession()?.fetched_at).toBeGreaterThanOrEqual(NOW));
    expect(calls.count).toBe(1);
  });

  it("refreshes when a stale tab becomes visible again", async () => {
    storeSession(Date.now());
    const calls = mockMe(() => HttpResponse.json(testSession()));
    const { useAuthState } = await loadAuthState();
    renderHook(useAuthState);
    expect(calls.count).toBe(0);

    vi.setSystemTime(NOW + FIVE_MINUTES);
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.waitFor(() => expect(calls.count).toBe(1));
  });

  it("identifies the user to the tracker when the snippet is present", async () => {
    const dispatch = vi.fn();
    (window as unknown as Record<string, unknown>).dispatch = dispatch;
    storeSession(Date.now());
    const { useAuthState } = await loadAuthState();
    renderHook(useAuthState);
    expect(dispatch).toHaveBeenCalledWith("metadata", {
      user_id: "user-1",
      user_email: "ada@example.com",
      account_id: "acct-1",
    });
  });
});

describe("reloadAuthSession", () => {
  it("fetches again after a refresh that started before the change", async () => {
    const before = testSession({ projects: { a_com: testProject("1") } });
    const after = testSession({ projects: { a_com: testProject("1"), b_com: testProject("2") } });
    let answer!: (response: Response) => void;
    let responses = 0;
    mockMe(() => {
      responses += 1;
      return responses === 1
        ? (new Promise<Response>((resolve) => {
            answer = resolve;
          }) as unknown as Response)
        : HttpResponse.json(after);
    });
    storeSession(0, before);
    const { useAuthState, reloadAuthSession } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    // The stale refresh from mounting is in flight when the project is added.
    await vi.waitFor(() => expect(answer).toBeTypeOf("function"));
    const reloaded = reloadAuthSession();
    answer(HttpResponse.json(before));
    expect(Object.keys((await reloaded)?.projects ?? {})).toEqual(["a_com", "b_com"]);
    expect(Object.keys(result.authUser()?.projects ?? {})).toEqual(["a_com", "b_com"]);
  });
});

describe("saveAuthSession", () => {
  it("does not notify subscribers when the account is unchanged", async () => {
    const { saveAuthSession, useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    saveAuthSession(testSession());
    const first = result.authUser();
    saveAuthSession(testSession());
    expect(result.authUser()).toBe(first);
  });
});

describe("patchProjectTimezone", () => {
  it("updates the matching project's zone in memory and storage", async () => {
    const { patchProjectTimezone, saveAuthSession, useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    saveAuthSession(testSession());
    patchProjectTimezone("42", "Europe/Berlin");
    expect(result.authUser()?.projects.example_com?.settings.timezone).toBe("Europe/Berlin");
    expect(storedSession().user.projects.example_com.settings.timezone).toBe("Europe/Berlin");
  });

  it("leaves the session alone for an unknown project or unchanged zone", async () => {
    const { patchProjectTimezone, saveAuthSession, useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    saveAuthSession(testSession());
    const before = result.authUser();
    patchProjectTimezone("404", "Europe/Berlin");
    patchProjectTimezone("42", "America/New_York");
    expect(result.authUser()).toBe(before);
  });
});

describe("logOut", () => {
  it("posts the logout with the cookie and clears the session", async () => {
    let credentials: RequestCredentials | undefined;
    server.use(
      http.post(`${API}/v2/public/auth/logout`, ({ request }) => {
        credentials = request.credentials;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { logOut, saveAuthSession, useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    saveAuthSession(testSession());
    await logOut();
    expect(credentials).toBe("include");
    expect(result.authUser()).toBeNull();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("clears the session even when the server can't be reached", async () => {
    server.use(http.post(`${API}/v2/public/auth/logout`, () => HttpResponse.error()));
    const { logOut, saveAuthSession, useAuthState } = await loadAuthState();
    const { result } = renderHook(useAuthState);
    saveAuthSession(testSession());
    await expect(logOut()).rejects.toThrow();
    expect(result.authUser()).toBeNull();
  });
});
