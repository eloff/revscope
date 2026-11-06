import { HttpResponse, http } from "msw";
import { setupServer } from "msw/node";

/** Origin `getApiBaseUrl()` resolves to under test (see `astroEnvClient.ts`). */
export const API = "http://localhost:4320";

/**
 * Tests register their routes with `server.use(...)`; handlers reset after each
 * test to these defaults. Every login and signup form asks which sign-in
 * providers are on, so by default none are.
 */
export const server = setupServer(
  http.get(`${API}/v2/public/auth/providers`, () =>
    HttpResponse.json({ google: null, apple: null }),
  ),
);
