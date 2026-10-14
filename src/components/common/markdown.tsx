import { memo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark, oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";
import { Check, Copy } from "lucide-react";
import { copyText, preprocessLatex } from "../../lib/utils";

function CodeBlock({ language, code, theme }: { language: string; code: string; theme: "light" | "dark" }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await copyText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch { /* clipboard unavailable */ }
  }
  return <div className="code-block"><button type="button" className={copied ? "code-copy copied" : "code-copy"} title={copied ? "Copied" : "Copy code"} onClick={() => void copy()}>{copied ? <Check size={13} /> : <Copy size={13} />}</button><SyntaxHighlighter language={language} style={theme === "dark" ? oneDark : oneLight} showLineNumbers wrapLongLines customStyle={{ margin: "0", background: "transparent", padding: "12px 0" }}>{code}</SyntaxHighlighter></div>;
}

export function Markdown({ content, theme }: { content: string; theme: "light" | "dark" }) {
  return <ReactMarkdown remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex]} components={{ code({ className, children, ...props }) { const language = /language-(\w+)/.exec(className ?? "")?.[1]; const source = String(children).replace(/\n$/, ""); return language ? <CodeBlock language={language} code={source} theme={theme} /> : <code className={className} {...props}>{children}</code>; } }}>{preprocessLatex(content)}</ReactMarkdown>;
}

export const MemoMarkdown = memo(Markdown);
