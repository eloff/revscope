/// <reference types="solid-js" />

import { Menu } from "@ark-ui/solid/menu";
import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import {
  avatar,
  chipProject,
  chipSeparator,
  menuContent,
  menuIcon,
  menuItem,
  menuItemIndicatorSlot,
  menuItemText,
  menuPositioner,
  menuSeparator,
  userChip,
  userName,
} from "./AuthNav.css";
import { logOut, useAuthState } from "./authState";
import { appPath, docsUrl, SELF_HOSTED } from "./links";
import {
  type ProjectOption,
  projectOptions,
  resolveProjectId,
  selectedProjectId,
  setSelectedProjectId,
} from "./projectSelection";

export type LoggedInUserProps = {
  projects?: ProjectOption[];
  projectId?: string;
  onProjectChange?: (projectId: string) => void;
};

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean).slice(0, 2);
  if (parts.length === 0) {
    return "?";
  }
  return parts.map((part) => part[0]?.toUpperCase() ?? "").join("");
}

function firstName(name: string): string {
  return name.trim().split(/\s+/).filter(Boolean)[0] ?? name;
}

function HamburgerIcon() {
  return (
    <svg
      class={menuIcon}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M2.5 4h11" />
      <path d="M2.5 8h11" />
      <path d="M2.5 12h11" />
    </svg>
  );
}

function isDashboardRoute(): boolean {
  const path = window.location.pathname;
  const dashboard = appPath("/dashboard");
  return path === dashboard || path.startsWith(`${dashboard}/`);
}

export default function LoggedInUser(props: LoggedInUserProps) {
  const { authUser, authResolved } = useAuthState();
  // Sibling client:only islands may resolve the shared auth session before this
  // island hydrates. Stay empty until mount so the first client paint matches SSR
  // (auth starts unresolved) and Solid doesn't abort the rest of the tree.
  const [mounted, setMounted] = createSignal(false);
  onMount(() => setMounted(true));

  // Always surface the current project in the chip and menu, even when the
  // account only has one.
  const projects = createMemo<ProjectOption[]>(() => {
    const passed = props.projects;
    if (passed != null && passed.length > 0) {
      return passed;
    }
    const user = authUser();
    return user ? projectOptions(user.projects) : [];
  });

  const projectId = createMemo(() => {
    const passed = props.projectId;
    if (passed) {
      return passed;
    }
    return resolveProjectId(projects(), selectedProjectId());
  });

  const selectedProjectLabel = createMemo(
    () => projects().find((project) => project.value === projectId())?.label ?? "",
  );

  const selectProject = (nextProjectId: string) => {
    if (!nextProjectId || nextProjectId === projectId()) {
      return;
    }
    if (props.onProjectChange) {
      props.onProjectChange(nextProjectId);
      return;
    }
    setSelectedProjectId(nextProjectId);
  };

  return (
    <Show when={mounted() && authResolved() && authUser()}>
      {(currentUser) => (
        <Menu.Root positioning={{ placement: "bottom-end", gutter: 6 }}>
          <Menu.Trigger class={userChip} aria-label="Account menu">
            {currentUser().gravatar_url ? (
              <img
                class={avatar}
                src={currentUser().gravatar_url}
                alt=""
                loading="lazy"
                decoding="async"
              />
            ) : (
              <div class={avatar} aria-hidden="true">
                {initials(currentUser().name)}
              </div>
            )}
            <span class={userName}>
              <span>{firstName(currentUser().name)}</span>
              <Show when={selectedProjectLabel()}>
                {(label) => (
                  <>
                    <span class={chipSeparator} aria-hidden="true">
                      ·
                    </span>
                    <span class={chipProject}>{label()}</span>
                  </>
                )}
              </Show>
            </span>
            <HamburgerIcon />
          </Menu.Trigger>
          <Portal>
            <Menu.Positioner class={menuPositioner}>
              <Menu.Content class={menuContent}>
                <Show when={!isDashboardRoute()}>
                  <Menu.Item
                    value="dashboard"
                    class={menuItem}
                    onSelect={() => window.location.assign(appPath("/dashboard"))}
                  >
                    Dashboard
                  </Menu.Item>
                </Show>
                <Menu.Item
                  value="settings"
                  class={menuItem}
                  onSelect={() => window.location.assign(appPath("/settings"))}
                >
                  Settings
                </Menu.Item>
                <Show when={currentUser().is_admin}>
                  <Menu.Item
                    value="users"
                    class={menuItem}
                    onSelect={() => window.location.assign(appPath("/settings/users"))}
                  >
                    Users
                  </Menu.Item>
                </Show>
                <Show when={!SELF_HOSTED && currentUser().is_admin}>
                  <Menu.Item
                    value="billing"
                    class={menuItem}
                    onSelect={() => window.location.assign("/billing")}
                  >
                    Billing
                  </Menu.Item>
                </Show>
                <Menu.Item
                  value="troubleshooting"
                  class={menuItem}
                  onSelect={() => window.location.assign(appPath("/troubleshooting"))}
                >
                  Troubleshooting
                </Menu.Item>
                <Menu.Item
                  value="docs"
                  class={menuItem}
                  onSelect={() => window.location.assign(docsUrl("/docs"))}
                >
                  Docs
                </Menu.Item>
                <Menu.Separator class={menuSeparator} />
                <Menu.Item value="logout" class={menuItem} onSelect={() => void logOut()}>
                  Log out
                </Menu.Item>
                <Show when={projects().length > 0}>
                  <Menu.Separator class={menuSeparator} />
                  <Menu.RadioItemGroup
                    value={projectId()}
                    onValueChange={(details) => selectProject(details.value)}
                  >
                    <For each={projects()}>
                      {(project) => (
                        <Menu.RadioItem value={project.value} class={menuItem}>
                          <Menu.ItemText class={menuItemText}>{project.label}</Menu.ItemText>
                          <span class={menuItemIndicatorSlot} aria-hidden="true">
                            <Menu.ItemIndicator>✓</Menu.ItemIndicator>
                          </span>
                        </Menu.RadioItem>
                      )}
                    </For>
                  </Menu.RadioItemGroup>
                </Show>
              </Menu.Content>
            </Menu.Positioner>
          </Portal>
        </Menu.Root>
      )}
    </Show>
  );
}
