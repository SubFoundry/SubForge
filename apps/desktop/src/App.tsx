import { Link, Outlet, useLocation } from "react-router-dom";

const navItems = [
  { path: "/", label: "Dashboard" },
  { path: "/plugins", label: "Plugins" },
  { path: "/sources", label: "Sources" },
  { path: "/profiles", label: "Profiles" },
  { path: "/runs", label: "Runs" },
  { path: "/settings", label: "Settings" },
];

export default function App() {
  const location = useLocation();

  return (
    <div className="min-h-screen text-slate-100">
      <div className="mx-auto grid min-h-screen max-w-7xl grid-cols-[220px_1fr] gap-6 p-6">
        <aside className="rounded-2xl border border-slate-800/80 bg-slate-950/65 p-4 backdrop-blur">
          <h1 className="mb-6 text-xl font-semibold tracking-wide">SubForge</h1>
          <nav className="space-y-2">
            {navItems.map((item) => {
              const active =
                item.path === "/"
                  ? location.pathname === "/"
                  : location.pathname.startsWith(item.path);
              return (
                <Link
                  key={item.path}
                  to={item.path}
                  className={`block rounded-lg px-3 py-2 text-sm transition ${
                    active
                      ? "bg-cyan-500/25 text-cyan-100"
                      : "text-slate-300 hover:bg-slate-800/80"
                  }`}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>
        </aside>

        <main className="rounded-2xl border border-slate-800/80 bg-slate-950/50 p-6 backdrop-blur">
          <Outlet />
        </main>
      </div>
    </div>
  );
}