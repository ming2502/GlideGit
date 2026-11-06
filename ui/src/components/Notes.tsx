// Release notes (a CHANGELOG section) rendered as formatted text.
import type { ReactNode } from "react";
import { changelogBlocks, inlineSpans } from "../lib/changelog";

function inline(text: string): ReactNode[] {
  return inlineSpans(text).map((s, i) =>
    s.kind === "code" ? <code key={i}>{s.text}</code> : s.kind === "strong" ? <strong key={i}>{s.text}</strong> : s.text,
  );
}

export function Notes({ markdown, className = "notes" }: { markdown: string; className?: string }) {
  const blocks = changelogBlocks(markdown);
  const out: ReactNode[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i];
    if (b.kind === "heading") out.push(<h4 key={i}>{inline(b.text)}</h4>);
    else if (b.kind === "text") out.push(<p key={i}>{inline(b.text)}</p>);
    else {
      // Consecutive items make one list.
      const items: ReactNode[] = [];
      for (; i < blocks.length && blocks[i].kind === "item"; i++) items.push(<li key={i}>{inline(blocks[i].text)}</li>);
      i--;
      out.push(<ul key={`l${i}`}>{items}</ul>);
    }
  }
  return <div className={className}>{out}</div>;
}
