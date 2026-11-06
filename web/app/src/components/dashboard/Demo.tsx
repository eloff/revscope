/// <reference types="solid-js" />
import { DEMO_PROJECT_ID } from "../../lib/dashboardApi";
import DashboardShell from "./DashboardShell";

export default function Demo() {
  return (
    <DashboardShell
      projectId={DEMO_PROJECT_ID}
      publicAccess
      layout="fullscreen"
      showHeader={false}
    />
  );
}
