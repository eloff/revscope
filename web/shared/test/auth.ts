import { HttpResponse, http } from "msw";
import { logOut, saveAuthSession } from "../src/authState";
import { testSession } from "./fixtures";
import { API, server } from "./server";

/** Signs the shared auth signals in, as a successful login would. */
export function signIn(session = testSession()) {
  saveAuthSession(session);
  return session;
}

/** authState's signals are module-level, so tests sharing a module sign out between runs. */
export async function signOut() {
  server.use(http.post(`${API}/v2/public/auth/logout`, () => new HttpResponse(null)));
  await logOut();
}
