import { useMemo } from "react";
import type { DiffLine } from "../bindings/DiffLine";
import { VirtualList } from "../components/VirtualList";
import { splitPath } from "../lib/format";
import { useRepo } from "../stores/repo";

type Row = { type: "hunk"; header: string } | { type: "line"; line: DiffLine };

const ROW_H = 20;

export function DiffView() {
  const diff = useRepo((s) => s.diff);
  const loading = useRepo((s) => s.diffLoading);
  const selectedPath = useRepo((s) => s.selectedPath);

  const rows = useMemo(() => {
    const out: Row[] = [];
    for (const h of diff?.hunks ?? []) {
      out.push({ type: "hunk", header: h.header });
      for (const l of h.lines) out.push({ type: "line", line: l });
    }
    return out;
  }, [diff]);

  if (!selectedPath) {
    return (
      <div className="diff-empty">
        <div>选择一个文件查看差异</div>
        <div className="muted">↑↓ 切换文件 · 空格 加入/排除提交</div>
      </div>
    );
  }

  const { name, dir } = splitPath(selectedPath);

  return (
    <div className="diff-view">
      <div className="diff-header">
        <span className="diff-file">{name}</span>
        <span className="muted">{dir}</span>
        <span className="spacer" />
        {loading && <span className="spinner" />}
        {diff && (
          <span className="diff-stats">
            <span className="add">+{diff.added}</span> <span className="del">−{diff.deleted}</span>
          </span>
        )}
        <span className="muted">HEAD ↔ 工作区</span>
      </div>
      {diff?.binary ? (
        <div className="diff-empty">二进制文件，无法显示文本差异</div>
      ) : diff && diff.hunks.length === 0 && !loading ? (
        <div className="diff-empty">{diff.truncated ? "文件过大，未加载预览" : "内容无差异（可能仅文件模式变化）"}</div>
      ) : (
        <>
          <VirtualList
            items={rows}
            rowHeight={ROW_H}
            className="diff-body"
            render={(row, i) =>
              row.type === "hunk" ? (
                <div key={i} className="dl hunk" style={{ height: ROW_H }}>
                  <span className="ln" />
                  <span className="ln" />
                  <span className="code">{row.header}</span>
                </div>
              ) : (
                <div key={i} className={`dl ${row.line.kind}`} style={{ height: ROW_H }}>
                  <span className="ln">{row.line.oldLine ?? ""}</span>
                  <span className="ln">{row.line.newLine ?? ""}</span>
                  <span className="sign">
                    {row.line.kind === "added" ? "+" : row.line.kind === "deleted" ? "−" : row.line.kind === "noNewline" ? "\\" : " "}
                  </span>
                  <span className="code">{row.line.text}</span>
                </div>
              )
            }
          />
          {diff?.truncated && <div className="diff-trunc">差异过长，已截断显示</div>}
        </>
      )}
    </div>
  );
}
