/// <reference types="solid-js" />
import { useAuthState } from "@revscope/web-shared/authState";
import {
  type Accessor,
  createContext,
  createResource,
  type ParentProps,
  Show,
  useContext,
} from "solid-js";
import { fetchImportedCoverage, type ImportedCoverage } from "../../lib/importedCoverageApi";
import { hasSegmentFilters } from "../../lib/segmentFilters";
import { useDashboardState } from "./dashboardPeriodContext";
import type { DashboardTab } from "./dashboardTabs";
import { importedNotice } from "./ImportedCoverageContext.css";
import { importedDataLeftOut } from "./importedCoverage";

const ImportedCoverageContext = createContext<Accessor<ImportedCoverage | undefined>>(
  () => undefined,
);

type ProviderProps = ParentProps<{
  projectId: string;
  /** The public demo has no imports, and no session to ask with. */
  publicAccess: boolean;
}>;

/** What's been imported for the shown project and range, for the views that say so. */
export function ImportedCoverageProvider(props: ProviderProps) {
  const { authUser } = useAuthState();
  const { range, rangeReady } = useDashboardState();
  const [coverage] = createResource(
    () => {
      const user = authUser();
      return user && !props.publicAccess && rangeReady()
        ? { user, projectId: props.projectId, range: range() }
        : null;
    },
    // A failed check only costs the shading and the notice, so reports carry on.
    ({ user, projectId, range }) =>
      fetchImportedCoverage(user, projectId, range.start, range.end).catch(() => undefined),
  );
  // Reading `.latest` keeps the previous answer on screen while a new range loads,
  // and doesn't trip the page's Suspense boundary.
  return (
    <ImportedCoverageContext.Provider value={() => coverage.latest}>
      {props.children}
    </ImportedCoverageContext.Provider>
  );
}

export function useImportedCoverage(): Accessor<ImportedCoverage | undefined> {
  return useContext(ImportedCoverageContext);
}

/** Shown only when this view's range has imported data it isn't counting. */
export function ImportedDataNotice(props: { tab: DashboardTab }) {
  const coverage = useImportedCoverage();
  const { segments } = useDashboardState();
  return (
    <Show when={importedDataLeftOut(coverage(), props.tab, hasSegmentFilters(segments()))}>
      <p class={importedNotice} role="status">
        Imported Google Analytics data isn't included in this view.
      </p>
    </Show>
  );
}
