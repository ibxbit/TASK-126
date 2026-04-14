import { useState } from "react";
import { login, LoginResponse } from "../ipc/auth";

interface Props {
  onLogin: (user: LoginResponse) => void;
}

export default function LoginForm({ onLogin }: Props) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const resp = await login(username, password);
      onLogin(resp);
    } catch (err: unknown) {
      const msg =
        typeof err === "string"
          ? err
          : err && typeof err === "object" && "message" in err
            ? String((err as { message: string }).message)
            : "Login failed";
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  return (
    <main style={styles.root}>
      <form onSubmit={submit} style={styles.card}>
        <h1 style={styles.title}>Shoreline Property Operations</h1>
        <p style={styles.subtitle}>Sign in to continue</p>
        <label style={styles.label}>
          Username
          <input
            style={styles.input}
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoFocus
            required
          />
        </label>
        <label style={styles.label}>
          Password
          <input
            style={styles.input}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        {error && <div style={styles.error}>{error}</div>}
        <button type="submit" style={styles.button} disabled={loading}>
          {loading ? "Signing in..." : "Sign in"}
        </button>
      </form>
    </main>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    fontFamily: "Segoe UI, -apple-system, Arial, sans-serif",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    minHeight: "100vh",
    background: "#f0f2f5",
  },
  card: {
    background: "#fff",
    borderRadius: 8,
    padding: 32,
    width: 360,
    boxShadow: "0 2px 8px rgba(0,0,0,0.1)",
    display: "flex",
    flexDirection: "column",
    gap: 16,
  },
  title: { fontSize: 20, margin: 0 },
  subtitle: { fontSize: 13, color: "#555", margin: 0 },
  label: { display: "flex", flexDirection: "column", fontSize: 13, fontWeight: 500, gap: 4 },
  input: {
    padding: "8px 10px",
    border: "1px solid #ccc",
    borderRadius: 4,
    fontSize: 14,
    outline: "none",
  },
  error: { color: "#c00", fontSize: 13 },
  button: {
    padding: "10px 0",
    background: "#1a73e8",
    color: "#fff",
    border: "none",
    borderRadius: 4,
    fontSize: 14,
    fontWeight: 600,
    cursor: "pointer",
    marginTop: 4,
  },
};
