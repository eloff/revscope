/// <reference types="solid-js" />

import { primaryButton } from "@revscope/web-shared/authFormStyles";
import { useAuthState } from "@revscope/web-shared/authState";
import LoginDialog from "@revscope/web-shared/LoginDialog";
import { SELF_HOSTED } from "@revscope/web-shared/links";
import {
  isDemoProjectId,
  type ProjectOption,
  projectOptions,
  resolveProjectId,
  selectedProjectId,
  setSelectedProjectId,
} from "@revscope/web-shared/projectSelection";
import { createEffect, createMemo, createSignal, Show, untrack } from "solid-js";
import { DEMO_PROJECT_ID } from "../../lib/dashboardApi";
import type { DateRangeSeconds } from "../../lib/dateRange";
import DashboardShell from "./DashboardShell";
import { page } from "./DashboardShell.css";
import { clearDashboardTargetUrlParams } from "./dashboardRoute";
import { message } from "./messages.css";
import type { ProjectOverviewCache } from "./ProjectOverviewCard";

export default function Dashboard() {
  const { authUser, authResolved } = useAuthState();

  /**
   * Read straight off the cached account so the first paint has the project
   * list, and so a background `/v2/session/me` refresh that adds or renames a
   * project shows up here without a reload.
   */
  const projects = createMemo<ProjectOption[]>(() => {
    const user = authUser();
    return user ? projectOptions(user.projects) : [];
  });
  const loadError = () =>
    authUser() && projects().length === 0 ? "This account has no projects yet." : "";

  // Visitors without a session get the public demo project; there's nothing
  // else for them to look at. A self-hosted instance's demo project is empty,
  // so its visitors are asked to log in instead.
  const needsLogin = () => SELF_HOSTED && authResolved() && !authUser();
  const shellProjectId = () => {
    if (authUser()) {
      return resolveProjectId(projects(), selectedProjectId());
    }
    return SELF_HOSTED ? "" : DEMO_PROJECT_ID;
  };
  const [loginOpen, setLoginOpen] = createSignal(true);

  const projectOverviewCache: ProjectOverviewCache = new Map();

  // Records the fallback as the selection, so a project added later that sorts
  // first doesn't take over the dashboard (and remount it mid-dialog).
  createEffect(() => {
    if (!authUser()) {
      return;
    }
    const resolved = shellProjectId();
    if (resolved && resolved !== selectedProjectId()) {
      setSelectedProjectId(resolved);
    }
  });

  const [carriedRange, setCarriedRange] = createSignal<DateRangeSeconds | undefined>();
  const selectProject = (projectId: string, range?: DateRangeSeconds) => {
    clearDashboardTargetUrlParams();
    setCarriedRange(range);
    setSelectedProjectId(projectId);
  };

  return (
    <Show
      when={authResolved() ? shellProjectId() : ""}
      keyed
      fallback={
        <div class={page}>
          <Show
            when={needsLogin()}
            fallback={<p class={message}>{loadError() || "Loading analytics…"}</p>}
          >
            <p class={message}>Log in to view analytics.</p>
            <button type="button" class={primaryButton} onClick={() => setLoginOpen(true)}>
              Log in
            </button>
            <LoginDialog
              open={loginOpen()}
              onOpenChange={setLoginOpen}
              redirectTo={`${window.location.pathname}${window.location.search}`}
            />
          </Show>
        </div>
      }
    >
      {/* Keyed so switching projects remounts the shell, dropping every
          cache and filter that named rows of the previous project. */}
      {(projectId) => (
        <DashboardShell
          projectId={projectId}
          // Demo reads stay on the public APIs whether or not the viewer is
          // signed in; Ask still uses the session when one is present.
          publicAccess={!authUser() || isDemoProjectId(projectId)}
          projects={projects()}
          onProjectChange={selectProject}
          projectOverviewCache={projectOverviewCache}
          initialRange={untrack(carriedRange)}
        />
      )}
    </Show>
  );
}
