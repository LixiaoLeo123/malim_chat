import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";
// rehype-katex renders math with katex@^0.16, so this CSS must come from the same major:
// newer KaTeX renamed the layout classes (sizing/strut/base -> katex-sizing/...) and a
// mismatch leaves fraction contents and exponents unscaled and misplaced. Keep katex pinned to ^0.16.
import "katex/dist/katex.min.css";

createRoot(document.getElementById("root")!).render(<StrictMode><App /></StrictMode>);
