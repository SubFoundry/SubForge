import { createBrowserRouter, RouterProvider } from "react-router-dom";
import App from "./App";
import DashboardPage from "./features/dashboard/page";
import PluginsPage from "./features/plugins/page";
import SourcesPage from "./features/sources/page";
import ProfilesPage from "./features/profiles/page";
import RunsPage from "./features/runs/page";
import SettingsPage from "./features/settings/page";

const router = createBrowserRouter([
  {
    path: "/",
    element: <App />,
    children: [
      { index: true, element: <DashboardPage /> },
      { path: "plugins", element: <PluginsPage /> },
      { path: "sources", element: <SourcesPage /> },
      { path: "profiles", element: <ProfilesPage /> },
      { path: "runs", element: <RunsPage /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
]);

export function AppRouter() {
  return <RouterProvider router={router} />;
}