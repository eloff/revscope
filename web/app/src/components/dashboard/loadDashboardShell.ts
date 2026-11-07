import { createOnceLoader } from "../../lib/onceLoader";

/**
 * The full dashboard, fetched on demand so the home page embed doesn't download
 * it up front. Nothing the embed loads may import DashboardShellContent
 * statically, or the bundler folds it back into the embed's own chunk.
 */
export const loadDashboardShell = createOnceLoader(() => import("./DashboardShellContent"));
