/**
 * Wraps a loader so concurrent and repeated calls share one in-flight load.
 * A failed load is forgotten, so a later call can try again.
 */
export function createOnceLoader<T>(load: () => Promise<T>): () => Promise<T> {
  let pending: Promise<T> | undefined;
  return () => {
    pending ??= load().catch((error: unknown) => {
      pending = undefined;
      throw error;
    });
    return pending;
  };
}
