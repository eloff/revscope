/// <reference types="solid-js" />
import { useAuthState } from "@revscope/web-shared/authState";
import { projectTimeZone } from "@revscope/web-shared/projectSelection";
import DashboardShellContent, { type DashboardShellProps } from "./DashboardShellContent";
import { DashboardStateProvider } from "./dashboardPeriodContext";

export default function DashboardShell(props: DashboardShellProps) {
  const { authUser } = useAuthState();
  // The server holds the account's own projects to its retention, and a public
  // project someone else owns, such as the demo, to none.
  const retentionMonths = () => {
    const user = authUser();
    const ownProject =
      !props.publicAccess &&
      Object.values(user?.projects ?? {}).some((project) => project.id === props.projectId);
    return ownProject ? (user?.data_retention_months ?? null) : null;
  };
  return (
    <DashboardStateProvider
      timeZone={() => projectTimeZone(props.projects, props.projectId)}
      initialRange={props.initialRange}
      retentionMonths={retentionMonths}
    >
      <DashboardShellContent {...props} />
    </DashboardStateProvider>
  );
}
