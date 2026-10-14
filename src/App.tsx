/**
 * Application entry component. The monolith that lived here was split into feature
 * modules under src/features — the chat shell (session gating, error boundary) is now
 * the Chat feature's exported root.
 */
export { App } from "./features/chat/Chat";
