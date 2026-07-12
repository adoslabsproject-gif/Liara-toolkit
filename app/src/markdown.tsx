// Rendering dei messaggi dell'assistente: markdown ricco (tabelle, codice evidenziato, link, grafici)
// e isolamento del blocco di ragionamento <think> in un bubble a parte. Estratto da App.tsx.
import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import "highlight.js/styles/github-dark.css";
import { ChartView } from "./Chart";

// SICUREZZA (XSS anti prompt-injection): l'output del modello può contenere HTML derivato da
// contenuto web non fidato (web_fetch/web_search). `rehypeRaw` lo trasforma in DOM reale, quindi
// DEVE essere ripulito: `rehypeSanitize` toglie <script>, i gestori inline (onerror/onclick…) e le
// URL javascript:, lasciando SOLO i tag di layout che il system prompt chiede al modello (tabelle,
// div/span, grassetto, con `style`/`className` inline). Difesa in profondità con la CSP severa
// (tauri.conf.json): anche se un tag sfuggisse, `script-src 'self'` blocca comunque l'esecuzione.
const SANITIZE_SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    "*": [...(defaultSchema.attributes?.["*"] ?? []), "className", "style"],
  },
  tagNames: [
    ...(defaultSchema.tagNames ?? []),
    "span", "div", "table", "thead", "tbody", "tfoot", "tr", "td", "th", "b", "i", "u", "small", "mark",
  ],
};

// Ragionamento (thinking di Qwen3, ON su desktop): bubble collassabile, separato dalla risposta.
function ThinkBubble({ text, live }: { text: string; live?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="think">
      <button className="think-head" onClick={() => setOpen((o) => !o)}>💭 {live ? "Sto ragionando…" : "Ragionamento"}<span className="think-caret">{open ? "▲" : "▼"}</span></button>
      {open && <div className="think-body">{text}</div>}
    </div>
  );
}

// rich rendering of assistant messages: tables, code (highlighted), lists, links, charts
function Md({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeRaw, [rehypeSanitize, SANITIZE_SCHEMA], rehypeHighlight]}
      components={{
        a: ({ href, children }) => (
          <a href={href} onClick={(e) => { e.preventDefault(); if (href) openUrl(href).catch(() => {}); }}>{children}</a>
        ),
        // ```chart {json}``` → a real interactive chart instead of a code block
        pre: (props) => {
          const kids = props.children as { props?: { className?: string; children?: unknown } } | { props?: { className?: string; children?: unknown } }[];
          const child = Array.isArray(kids) ? kids[0] : kids;
          const cls = child?.props?.className || "";
          if (cls.includes("language-chart")) {
            return <ChartView raw={String(child?.props?.children ?? "").replace(/\n$/, "")} />;
          }
          return <pre>{props.children}</pre>;
        },
      }}
    >
      {text}
    </ReactMarkdown>
  );
}

// Isola l'eventuale blocco <think>...</think> (thinking ON) in un bubble a parte; il resto è Markdown.
export function AssistantBody({ text }: { text: string }) {
  const closed = text.match(/^\s*<think>([\s\S]*?)<\/think>\s*([\s\S]*)$/);
  if (closed) {
    const reasoning = closed[1].trim();
    return (<>{reasoning && <ThinkBubble text={reasoning} />}<Md text={closed[2]} /></>);
  }
  const openThink = text.match(/^\s*<think>([\s\S]*)$/);
  if (openThink) return <ThinkBubble text={openThink[1].trim()} live />;
  return <Md text={text} />;
}
