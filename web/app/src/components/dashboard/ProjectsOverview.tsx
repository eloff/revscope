/// <reference types="solid-js" />
import type { AuthSession } from "@revscope/web-shared/authState";
import { SELF_HOSTED } from "@revscope/web-shared/links";
import {
  type ProjectOption,
  planAllowsMultipleProjects,
} from "@revscope/web-shared/projectSelection";
import { createSignal, For, Show } from "solid-js";
import { periodInTimeZone } from "../../lib/dateRange";
import { buttonIcon, primaryButton } from "../buttons.css";
import AddProjectDialog from "./AddProjectDialog";
import { useDashboardState } from "./dashboardPeriodContext";
import { message } from "./messages.css";
import ProjectOverviewCard, { type ProjectOverviewCache } from "./ProjectOverviewCard";
import { grid, header, headerNote, headerTitle } from "./ProjectsOverview.css";

type ProjectsOverviewProps = {
  session: AuthSession;
  projects: ProjectOption[];
  currentProjectId: string;
  cache: ProjectOverviewCache;
  onOpenProject: (projectId: string) => void;
};

/** Visitors and revenue for every project on the account, over the selected period. */
export default function ProjectsOverview(props: ProjectsOverviewProps) {
  const { range, rangeReady, timeZone } = useDashboardState();
  const [adding, setAdding] = createSignal(false);
  const canAdd = () => planAllowsMultipleProjects(props.session.plan);

  return (
    <>
      <div class={header}>
        <h2 class={headerTitle}>
          {props.projects.length === 1 ? "1 project" : `${props.projects.length} projects`}
        </h2>
        {/* Adding projects and changing plans are for admins. */}
        <Show when={props.session.is_admin}>
          <Show
            when={canAdd()}
            fallback={
              <p class={headerNote}>
                Your plan includes one website.{" "}
                <Show when={!SELF_HOSTED}>
                  <a href="/billing">Upgrade to Growth</a> to add more.
                </Show>
              </p>
            }
          >
            <button type="button" class={primaryButton} onClick={() => setAdding(true)}>
              <svg class={buttonIcon} viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <path
                  d="M8 3v10M3 8h10"
                  stroke="currentColor"
                  stroke-width="1.75"
                  stroke-linecap="round"
                />
              </svg>
              Add project
            </button>
          </Show>
        </Show>
      </div>
      <Show when={rangeReady()} fallback={<p class={message}>Loading projects…</p>}>
        <div class={grid}>
          <For each={props.projects}>
            {(project) => (
              <ProjectOverviewCard
                session={props.session}
                project={project}
                range={periodInTimeZone(range(), timeZone(), project.timezone)}
                current={project.value === props.currentProjectId}
                cache={props.cache}
                onOpen={() => props.onOpenProject(project.value)}
              />
            )}
          </For>
        </div>
      </Show>
      <AddProjectDialog
        session={props.session}
        open={adding()}
        onClose={() => setAdding(false)}
        onOpenProject={props.onOpenProject}
      />
    </>
  );
}
