import { onTestFinished, vi } from "vitest";

type AppleResponse = {
  authorization: { id_token: string };
  user?: { name?: { firstName?: string; lastName?: string } };
};

/**
 * jsdom never runs Google's or Apple's scripts, so the sign-in modules' script
 * tags "load" at once and find these fakes in their place. The modules keep
 * what loaded for the rest of the file, so the fakes route through state here
 * rather than through each test's own objects.
 */
let googleCallback: ((response: { credential: string }) => void) | undefined;
let googleRenderButton: (...args: unknown[]) => void = () => {};
let appleSignIn: () => Promise<AppleResponse> = () => Promise.reject(new Error("no fake"));

function loadScriptsAtOnce() {
  const append = vi.spyOn(document.head, "append").mockImplementation((...nodes) => {
    for (const node of nodes) {
      if (node instanceof HTMLScriptElement) {
        queueMicrotask(() => node.onload?.(new Event("load")));
      }
    }
  });
  onTestFinished(() => append.mockRestore());
}

/** Google's script, whose button `signIn` then stands in for clicking. */
export function fakeGoogle() {
  const renderButton = vi.fn();
  googleRenderButton = renderButton;
  vi.stubGlobal("google", {
    accounts: {
      id: {
        initialize: (options: { callback: typeof googleCallback }) => {
          googleCallback = options.callback;
        },
        renderButton: (...args: unknown[]) => googleRenderButton(...args),
      },
    },
  });
  loadScriptsAtOnce();
  return {
    renderButton,
    signIn: (credential: string) => googleCallback?.({ credential }),
  };
}

/** Apple's script, whose popup answers with `response` (or rejects with it, when an error). */
export function fakeApple(response: AppleResponse | { error: string }) {
  const init = vi.fn();
  appleSignIn = () => ("error" in response ? Promise.reject(response) : Promise.resolve(response));
  vi.stubGlobal("AppleID", { auth: { init, signIn: () => appleSignIn() } });
  loadScriptsAtOnce();
  return { init };
}
