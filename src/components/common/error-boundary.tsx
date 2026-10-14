import { Component, type ReactNode } from "react";
import { Bot } from "lucide-react";

export class ChatErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error) { console.error("Chat render failure", error); }
  render() {
    if (this.state.failed) return <main className="recovery-screen"><div><Bot size={28} /><h1>Unable to render this chat</h1><button className="primary" onClick={() => window.location.reload()}>Reload chat</button></div></main>;
    return this.props.children;
  }
}
