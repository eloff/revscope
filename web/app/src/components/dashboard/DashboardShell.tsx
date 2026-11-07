/// <reference types="solid-js" />
import { projectTimeZone } from "@revscope/web-shared/projectSelection";
import DashboardShellContent, { type DashboardShellProps } from "./DashboardShellContent";
import { DashboardStateProvider } from "./dashboardPeriodContext";

export default function DashboardShell(props: DashboardShellProps) {
  return (
    <DashboardStateProvider
      timeZone={() => projectTimeZone(props.projects, props.projectId)}
      initialRange={props.initialRange}
    >
      <DashboardShellContent {...props} />
    </DashboardStateProvider>
  );
}
