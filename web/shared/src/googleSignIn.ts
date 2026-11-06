interface GoogleIdentity {
  accounts: {
    id: {
      initialize(options: {
        client_id: string;
        callback: (response: { credential: string }) => void;
      }): void;
      renderButton(parent: HTMLElement, options: Record<string, string | number>): void;
    };
  };
}

let identityServices: Promise<GoogleIdentity> | undefined;
/**
 * Google takes one callback per page, not one per button, so sign-ins go to the
 * newest button still mounted: the login dialog's while it covers the signup
 * form's, then the form's again once it closes.
 */
const credentialHandlers: Array<(idToken: string) => void> = [];

/** Sends Google sign-ins to `onToken` until the returned function is called. */
export function receiveGoogleSignIns(onToken: (idToken: string) => void): () => void {
  credentialHandlers.push(onToken);
  return () => {
    const index = credentialHandlers.lastIndexOf(onToken);
    if (index >= 0) {
      credentialHandlers.splice(index, 1);
    }
  };
}

/**
 * Google Identity Services, loaded and initialized on first use so visitors who
 * never see the button don't fetch it.
 */
function loadIdentityServices(clientId: string): Promise<GoogleIdentity> {
  identityServices ??= new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "https://accounts.google.com/gsi/client";
    script.async = true;
    const fail = () => {
      identityServices = undefined;
      script.remove();
      reject(new Error("couldn't load Google sign-in"));
    };
    script.onload = () => {
      // A portal or blocker can answer with something that loads but isn't Google's script.
      try {
        const google = (window as unknown as { google: GoogleIdentity }).google;
        google.accounts.id.initialize({
          client_id: clientId,
          callback: (response) => credentialHandlers.at(-1)?.(response.credential),
        });
        resolve(google);
      } catch {
        fail();
      }
    };
    script.onerror = fail;
    document.head.append(script);
  });
  return identityServices;
}

/** Google's button takes a width in pixels, between these. */
const MIN_BUTTON_WIDTH = 200;
const MAX_BUTTON_WIDTH = 400;

/**
 * Draws Google's sign-in button across `parent`. Its sign-ins go where
 * `receiveGoogleSignIns` says. Google only allows its own button in the browser.
 */
export async function renderGoogleButton(parent: HTMLElement, clientId: string): Promise<void> {
  const google = await loadIdentityServices(clientId);
  const width = Math.min(MAX_BUTTON_WIDTH, Math.max(MIN_BUTTON_WIDTH, parent.clientWidth));
  google.accounts.id.renderButton(parent, {
    theme: "outline",
    size: "large",
    text: "continue_with",
    logo_alignment: "center",
    width,
  });
}
