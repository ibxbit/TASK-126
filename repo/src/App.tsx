// Landing shell. Dispatches on the current pathname:
//   `/`                       → Dashboard with workspace launch buttons
//   `/workspace/move-out`     → move-out case workspace
//   `/workspace/parcel-queue` → parcel queue workspace
//   `/workspace/claims-inbox` → claims inbox workspace
//
// The Rust side opens additional windows at these routes via
// `cmd_open_workspace`; each instance renders the corresponding view.

import { useEffect, useState } from "react";
import { openWorkspace, Workspace } from "./ipc/desktop";
import { currentUser, LoginResponse, logout } from "./ipc/auth";
import LoginForm from "./components/LoginForm";

export default function App() {
  const path = usePathname();
  const [user, setUser] = useState<LoginResponse | null | undefined>(undefined);

  useEffect(() => {
    // Check if already logged in (e.g. workspace child window inherits session).
    currentUser().then(setUser).catch(() => setUser(null));
  }, []);

  // Workspace windows skip the login gate — they inherit the session.
  if (path.startsWith("/workspace/move-out")) return <WorkspaceView title="Move-Out Case" />;
  if (path.startsWith("/workspace/parcel-queue")) return <WorkspaceView title="Parcel Queue" />;
  if (path.startsWith("/workspace/claims-inbox")) return <WorkspaceView title="Claims Inbox" />;

  // Loading.
  if (user === undefined) return null;

  // Not logged in.
  if (!user) return <LoginForm onLogin={setUser} />;

  return <Dashboard user={user} onLogout={() => { void logout(); setUser(null); }} />;
}

function usePathname(): string {
  const [p, setP] = useState<string>(() => window.location.pathname);
  useEffect(() => {
    const onPop = () => setP(window.location.pathname);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  return p;
}

function Dashboard({ user, onLogout }: { user: LoginResponse; onLogout: () => void }) {
  const open = async (w: Workspace) => {
    try {
      await openWorkspace(w);
    } catch (err) {
      // Surface IPC errors clearly during development.
      // eslint-disable-next-line no-console
      console.error("openWorkspace failed", err);
    }
  };
  return (
    <main style={styles.root}>
      <header style={styles.header}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h1 style={styles.title}>Shoreline Property Operations Console</h1>
          <div style={{ fontSize: 12, color: "#555" }}>
            {user.username} ({user.role}){" "}
            <button onClick={onLogout} style={{ fontSize: 12, cursor: "pointer", marginLeft: 8 }}>Sign out</button>
          </div>
        </div>
        <p style={styles.subtitle}>Select a workspace to open in a new window.</p>
      </header>
      <section style={styles.grid}>
        <WorkspaceCard
          title="Move-Out Case"
          description="Track deposits, inspections, deductions, and settlement approvals."
          onOpen={() => open("move_out_case")}
        />
        <WorkspaceCard
          title="Parcel Queue"
          description="Check-in, check-out, and deliver parcels against the lifecycle state machine."
          onOpen={() => open("parcel_queue")}
        />
        <WorkspaceCard
          title="Claims Inbox"
          description="Resolve disputes with two-party confirmation and 72-hour response windows."
          onOpen={() => open("claims_inbox")}
        />
      </section>
      <footer style={styles.footer}>
        Offline-first · v0.1.0 · Tauri + React + Rust + SQLite
      </footer>
    </main>
  );
}

function WorkspaceCard(props: { title: string; description: string; onOpen: () => void }) {
  return (
    <button onClick={props.onOpen} style={styles.card}>
      <div style={styles.cardTitle}>{props.title}</div>
      <div style={styles.cardDesc}>{props.description}</div>
    </button>
  );
}

function WorkspaceView({ title }: { title: string }) {
  return (
    <main style={styles.root}>
      <header style={styles.header}>
        <h1 style={styles.title}>{title}</h1>
        <p style={styles.subtitle}>
          This workspace window is ready. Domain views attach here once SQLite
          repositories are wired to the existing service layer.
        </p>
      </header>
    </main>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    fontFamily: "Segoe UI, -apple-system, Arial, sans-serif",
    color: "#111",
    background: "#fafafa",
    minHeight: "100vh",
    padding: 48,
    boxSizing: "border-box",
  },
  header: { marginBottom: 32 },
  title: { fontSize: 24, margin: 0 },
  subtitle: { fontSize: 13, color: "#555", marginTop: 4 },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(320px, 1fr))",
    gap: 16,
  },
  card: {
    textAlign: "left",
    padding: 20,
    border: "1px solid #ddd",
    borderRadius: 8,
    background: "#fff",
    cursor: "pointer",
    font: "inherit",
    transition: "border-color 120ms",
  },
  cardTitle: { fontSize: 16, fontWeight: 600, marginBottom: 6 },
  cardDesc: { fontSize: 13, color: "#555", lineHeight: 1.45 },
  footer: { marginTop: 48, fontSize: 11, color: "#888" },
};
