interface AppleSignInResponse {
  authorization: { id_token: string };
  /** Only on the first sign-in to our app; Apple never sends the name again. */
  user?: { name?: { firstName?: string; lastName?: string } };
}

interface AppleIdAuth {
  auth: {
    init(options: {
      clientId: string;
      scope: string;
      redirectURI: string;
      usePopup: boolean;
    }): void;
    signIn(): Promise<AppleSignInResponse>;
  };
}

export type AppleSignIn = { idToken: string; name?: string };

let appleJs: Promise<void> | undefined;
/** Apple's script once initialized, for calling without an `await` between the click and the popup. */
let loadedAppleJs: AppleIdAuth | undefined;

/**
 * Apple's sign-in script, loaded and initialized ahead of the click: the popup
 * must open straight from the click, or the browser blocks it.
 */
export function loadAppleJs(servicesId: string): Promise<void> {
  appleJs ??= new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src =
      "https://appleid.cdn-apple.com/appleauth/static/jsapi/appleid/1/en_US/appleid.auth.js";
    script.async = true;
    const fail = () => {
      appleJs = undefined;
      script.remove();
      reject(new Error("couldn't load Apple sign-in"));
    };
    script.onload = () => {
      // A portal or blocker can answer with something that loads but isn't Apple's script.
      try {
        const apple = (window as unknown as { AppleID: AppleIdAuth }).AppleID;
        apple.auth.init({
          clientId: servicesId,
          scope: "name email",
          // Apple checks this against the Services ID's return URLs, though the
          // popup hands the token back itself.
          redirectURI: `${window.location.origin}/`,
          usePopup: true,
        });
        loadedAppleJs = apple;
        resolve();
      } catch {
        fail();
      }
    };
    script.onerror = fail;
    document.head.append(script);
  });
  return appleJs;
}

/** An ID token from Apple's popup (once `loadAppleJs` is done), or null if the visitor closed it. */
export async function appleSignIn(): Promise<AppleSignIn | null> {
  if (!loadedAppleJs) {
    throw new Error("Apple sign-in hasn't loaded");
  }
  try {
    const response = await loadedAppleJs.auth.signIn();
    const name = [response.user?.name?.firstName, response.user?.name?.lastName]
      .filter(Boolean)
      .join(" ");
    return { idToken: response.authorization.id_token, name: name || undefined };
  } catch (err) {
    const error = (err as { error?: string }).error;
    if (error === "popup_closed_by_user" || error === "user_cancelled_authorize") {
      return null;
    }
    throw err;
  }
}
