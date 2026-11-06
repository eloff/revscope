import { HttpResponse, http } from "msw";
import { API, server } from "./server";

export type CapturedRequest = {
  url: URL;
  method: string;
  credentials: RequestCredentials;
  headers: Headers;
  /** Parsed JSON body, the raw text when it isn't JSON, or null when empty. */
  body: unknown;
};

type Method = "get" | "post" | "put" | "patch" | "delete";
type Responder = (request: CapturedRequest) => Response | Promise<Response>;

/**
 * Answers `method path` on the test API origin and records every request.
 * `response` is sent as JSON with `status`, or called to build the response.
 */
export function mockApi(
  method: Method,
  path: string,
  response: Responder | object = {},
  status = 200,
): CapturedRequest[] {
  const calls: CapturedRequest[] = [];
  server.use(
    http[method](`${API}${path}`, async ({ request }) => {
      const text = await request.text();
      let body: unknown = null;
      if (text) {
        try {
          body = JSON.parse(text);
        } catch {
          body = text;
        }
      }
      const captured: CapturedRequest = {
        url: new URL(request.url),
        method: request.method,
        credentials: request.credentials,
        headers: request.headers,
        body,
      };
      calls.push(captured);
      return typeof response === "function"
        ? (response as Responder)(captured)
        : HttpResponse.json(response, { status });
    }),
  );
  return calls;
}

/** The query string as a plain object, for compact assertions. */
export function queryOf(request: CapturedRequest | undefined): Record<string, string> {
  return Object.fromEntries(request?.url.searchParams ?? []);
}
