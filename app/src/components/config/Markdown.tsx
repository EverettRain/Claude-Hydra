import { Check, Copy } from "lucide-react";
import { marked, type Token, type Tokens } from "marked";
import { useMemo, useState, type ReactNode } from "react";
import { useToast } from "@/components/ui/toast";

/**
 * Markdown from a session, turned into React elements from marked's tokens. Nothing is ever
 * inserted as HTML: raw HTML in a message shows as text, so a transcript (which holds web pages
 * and tool output) cannot run code in Hydra's window.
 */
export function Markdown({ text }: { text: string }) {
  const tokens = useMemo(() => marked.lexer(text, { gfm: true }), [text]);
  return <div className="md">{blocks(tokens)}</div>;
}

function blocks(tokens: Token[]): ReactNode[] {
  return tokens.map((t, i) => <Block key={i} token={t} />);
}

function Block({ token }: { token: Token }) {
  switch (token.type) {
    case "heading": {
      const t = token as Tokens.Heading;
      const Tag = (["h1", "h2", "h3", "h4", "h5", "h6"] as const)[Math.min(t.depth, 6) - 1];
      return <Tag>{inline(t.tokens)}</Tag>;
    }
    case "paragraph": return <p>{inline((token as Tokens.Paragraph).tokens)}</p>;
    case "text": {
      const t = token as Tokens.Text;
      return <p>{t.tokens ? inline(t.tokens) : t.text}</p>;
    }
    case "code": {
      const t = token as Tokens.Code;
      return <CodeBlock code={t.text} lang={t.lang} />;
    }
    case "blockquote": return <blockquote>{blocks((token as Tokens.Blockquote).tokens)}</blockquote>;
    case "list": {
      const t = token as Tokens.List;
      const items = t.items.map((item, i) => (
        <li key={i} className={item.task ? "task" : undefined}>
          {item.task && <input type="checkbox" checked={!!item.checked} readOnly aria-label={item.checked ? "已完成" : "未完成"} />}
          {listItem(item)}
        </li>
      ));
      return t.ordered ? <ol start={typeof t.start === "number" ? t.start : undefined}>{items}</ol> : <ul>{items}</ul>;
    }
    case "table": {
      const t = token as Tokens.Table;
      const align = (a: string | null) => (a ? { textAlign: a as "left" | "right" | "center" } : undefined);
      return (
        <div className="table-wrap">
          <table>
            <thead><tr>{t.header.map((c, i) => <th key={i} style={align(c.align)}>{inline(c.tokens)}</th>)}</tr></thead>
            <tbody>{t.rows.map((row, r) => <tr key={r}>{row.map((c, i) => <td key={i} style={align(c.align)}>{inline(c.tokens)}</td>)}</tr>)}</tbody>
          </table>
        </div>
      );
    }
    case "hr": return <hr />;
    case "html": return <p className="whitespace-pre-wrap">{(token as Tokens.HTML).text}</p>;
    case "space": return null;
    default: return "text" in token && typeof token.text === "string" ? <p>{token.text}</p> : null;
  }
}

/** A tight list item holds `text` tokens; a loose one holds paragraphs. */
function listItem(item: Tokens.ListItem): ReactNode[] {
  return item.tokens.filter((t) => t.type !== "checkbox").map((t, i) =>
    t.type === "text" ? <span key={i}>{(t as Tokens.Text).tokens ? inline((t as Tokens.Text).tokens!) : (t as Tokens.Text).text}</span> : <Block key={i} token={t} />);
}

function inline(tokens: Token[] = []): ReactNode[] {
  return tokens.map((t, i) => <Inline key={i} token={t} />);
}

function Inline({ token }: { token: Token }) {
  switch (token.type) {
    case "strong": return <strong>{inline((token as Tokens.Strong).tokens)}</strong>;
    case "em": return <em>{inline((token as Tokens.Em).tokens)}</em>;
    case "del": return <del>{inline((token as Tokens.Del).tokens)}</del>;
    case "codespan": return <code>{(token as Tokens.Codespan).text}</code>;
    case "br": return <br />;
    case "link": {
      const t = token as Tokens.Link;
      return <CopyLink href={t.href}>{inline(t.tokens)}</CopyLink>;
    }
    case "image": {
      const t = token as Tokens.Image;
      return <CopyLink href={t.href}>［图片：{t.text || t.href}］</CopyLink>;
    }
    case "text": {
      const t = token as Tokens.Text;
      return <>{t.tokens ? inline(t.tokens) : t.text}</>;
    }
    default: return <>{"text" in token && typeof token.text === "string" ? token.text : ""}</>;
  }
}

/**
 * Links are copied, not opened: the system browser does not go through the account's proxy, so
 * opening one would visit it from this computer's own address.
 */
function CopyLink({ href, children }: { href: string; children: ReactNode }) {
  const toast = useToast();
  const copy = () => navigator.clipboard.writeText(href).then(() => toast("已复制链接"), () => toast("无法复制链接", "error"));
  return <button type="button" className="md-link" title={`${href}\n点击复制链接`} onClick={copy}>{children}</button>;
}

export function CodeBlock({ code, lang }: { code: string; lang?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => navigator.clipboard.writeText(code).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); });
  return (
    <div className="md-code">
      <div className="md-code-bar">
        <span>{lang || "代码"}</span>
        <button type="button" onClick={copy} aria-label="复制代码">{copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}{copied ? "已复制" : "复制"}</button>
      </div>
      <pre className="selectable"><code>{code}</code></pre>
    </div>
  );
}
