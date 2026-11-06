import { HttpResponse } from "msw";

/** A streamed body delivered in exactly these chunks, so frames can straddle reads. */
export function streamOf(chunks: string[]): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(encoder.encode(chunk));
      }
      controller.close();
    },
  });
  return new HttpResponse(body, { headers: { "Content-Type": "text/event-stream" } });
}

export function frame(event: string, data: unknown): string {
  return `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}
