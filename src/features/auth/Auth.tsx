import { FormEvent, useState } from "react";
import { Bot, LoaderCircle } from "lucide-react";
import { api } from "../../api";
import type { Session } from "../../types";

export function Auth({ onSession }: { onSession: (session: Session) => void }) {
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try { onSession(mode === "login" ? await api.login(email, password) : await api.signup(email, password, name)); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to continue."); }
    finally { setBusy(false); }
  }
  return <main className="auth-shell"><section className="auth-brand"><div className="brand-mark"><Bot size={28} /></div><h1>malim_chat</h1><p>One secure place for the AI models your work depends on.</p></section><form className="auth-form" onSubmit={submit}><h2>{mode === "login" ? "Welcome back" : "Create your account"}</h2>{mode === "signup" && <label>Display name<input value={name} required maxLength={80} onChange={(event) => setName(event.target.value)} /></label>}<label>Email<input type="email" value={email} required autoComplete="email" onChange={(event) => setEmail(event.target.value)} /></label><label>Password<input type="password" value={password} minLength={12} required autoComplete={mode === "login" ? "current-password" : "new-password"} onChange={(event) => setPassword(event.target.value)} /></label>{error && <p className="form-error">{error}</p>}<button className="primary" disabled={busy}>{busy && <LoaderCircle className="spin" size={17} />}{mode === "login" ? "Sign in" : "Create account"}</button><button type="button" className="text-button" onClick={() => { setMode(mode === "login" ? "signup" : "login"); setError(null); }}>{mode === "login" ? "Need an account? Sign up" : "Already have an account? Sign in"}</button></form></main>;
}
