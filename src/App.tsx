import { Sidebar } from "./components/Sidebar";
import { AppProvider, useApp } from "./lib/store";
import { ToastProvider } from "./lib/toast";
import { DashboardPage } from "./pages/Dashboard";
import { InboxPage } from "./pages/Inbox";
import { PlanPage } from "./pages/Plan";
import { SettingsPage } from "./pages/Settings";

function Shell() {
  const { page } = useApp();
  return (
    <div className="flex h-full overflow-hidden">
      <Sidebar />
      <main className="min-w-0 flex-1 bg-canvas">
        {page === "inbox" && <InboxPage />}
        {page === "plan" && <PlanPage />}
        {page === "dashboard" && <DashboardPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <AppProvider>
        <Shell />
      </AppProvider>
    </ToastProvider>
  );
}
