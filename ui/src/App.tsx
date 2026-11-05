import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { api, onSnapshot, onTaskProgress, type AppInfo } from "./api";
import { installKeyHandler, useActions } from "./actions/registry";
import { ConfirmDialog, Toasts } from "./components/Overlays";
import { load, save } from "./lib/storage";
import { useRepo } from "./stores/repo";
import { BranchPopup } from "./views/BranchPopup";
import { pickRepository, StatusBar, Toolbar } from "./views/Chrome";
import { CommitPanel } from "./views/CommitPanel";
import { DiffView } from "./views/DiffView";
import { LogView } from "./views/LogView";
import { StashDialog, StashView } from "./views/StashView";
import { RebaseDialog } from "./views/RebaseDialog";

// The merge tool brings CodeMirror; load it when first needed.
const CompareView = lazy(() => import("./views/CompareView").then((m) => ({ default: m.CompareView })));
const HistoryView = lazy(() => import("./views/HistoryView").then((m) => ({ default: m.HistoryView })));
const AnnotateView = lazy(() => import("./views/AnnotateView").then((m) => ({ default: m.AnnotateView })));
const MergeTool = lazy(() => import("./views/MergeTool").then((m) => ({ default: m.MergeTool })));
import { AskpassDialog } from "./views/AskpassDialog";
import { OperationBanner } from "./views/OperationBanner";
import { PushDialog } from "./views/PushDialog";
import { TabBar } from "./views/TabBar";
import { UndoCenter } from "./views/UndoCenter";
import { CommandPalette, installDoubleShift } from "./views/CommandPalette";
import { Welcome } from "./views/Welcome";

export function App() {
  const root = useRepo((s) => s.root);
  const view = useRepo((s) => s.view);
  const mergePath = useRepo((s) => s.mergePath);
  const annotate = useRepo((s) => s.annotate);
  const history = useRepo((s) => s.history);
  const compare = useRepo((s) => s.compare);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [restored, setRestored] = useState(false);

  useEffect(() => installKeyHandler(), []);
  useEffect(() => installDoubleShift(() => useRepo.getState().setPalette(true)), []);

  useEffect(() => {
    api.appInfo().then(setInfo).catch(() => {});
    void useRepo
      .getState()
      .restoreTabs()
      .finally(() => setRestored(true));
    const un = onSnapshot((r, s) => useRepo.getState().applySnapshot(r, s));
    const unProgress = onTaskProgress((id, p) => useRepo.getState().setTaskProgress(id, p));
    // Diff content can change without a status change; refresh when coming back.
    const onFocus = () => {
      const st = useRepo.getState();
      if (st.root) void st.reloadDiff();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      void un.then((f) => f());
      void unProgress.then((f) => f());
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  useActions([
    {
      id: "palette.open",
      title: "Find Action…",
      keys: ["mod+shift+a", "mod+shift+p"],
      hidden: true,
      run: () => useRepo.getState().setPalette(true),
    },
    { id: "repo.open", keywords: "打开仓库 open project", title: "Open Repository…", keys: ["mod+o"], run: () => void pickRepository() },
    {
      id: "repo.refresh",
      keywords: "刷新 reload",
      title: "Refresh",
      keys: ["mod+r", "f5"],
      enabled: () => !!useRepo.getState().root,
      run: () => {
        const st = useRepo.getState();
        void st.runTask("刷新", async (r) => st.applySnapshot(r, await api.refresh(r)));
      },
    },
    {
      id: "branches.popup",
      keywords: "分支 切换分支 checkout branch",
      title: "Branches…",
      keys: ["mod+shift+backquote", "mod+b"],
      enabled: () => !!useRepo.getState().root,
      run: () => {
        const st = useRepo.getState();
        st.setBranchPopup(!st.branchPopup);
      },
    },
    {
      id: "tabs.next",
      keywords: "下一个仓库标签",
      title: "Next Repository Tab",
      keys: ["mod+shift+bracketright", "ctrl+tab"],
      run: () => useRepo.getState().cycleTab(1),
    },
    {
      id: "tabs.prev",
      keywords: "上一个仓库标签",
      title: "Previous Repository Tab",
      keys: ["mod+shift+bracketleft", "ctrl+shift+tab"],
      run: () => useRepo.getState().cycleTab(-1),
    },
    {
      id: "tabs.close",
      keywords: "关闭仓库标签",
      title: "Close Repository Tab",
      keys: ["mod+alt+w"],
      enabled: () => !!useRepo.getState().root,
      run: () => void useRepo.getState().closeRepo(),
    },
    {
      id: "git.undoCenter",
      keywords: "撤销中心 操作记录 undo history oplog",
      title: "Undo Center…",
      keys: ["mod+shift+u"],
      run: () => useRepo.getState().setUndoCenter(true),
      enabled: () => !!useRepo.getState().root,
    },
    { id: "view.changes", keywords: "提交视图 变更 changes", title: "Go to Commit View", keys: ["mod+0"], run: () => useRepo.getState().setView("changes") },
    { id: "view.log", keywords: "日志 历史 提交历史 history", title: "Go to Log", keys: ["mod+9"], run: () => useRepo.getState().setView("log") },
    { id: "view.stash", keywords: "储藏 搁置 shelf", title: "Go to Stash / Shelf", keys: ["mod+8"], run: () => useRepo.getState().setView("stash") },
    {
      id: "git.stash",
      keywords: "储藏修改 stash",
      title: "Stash Changes…",
      keys: ["mod+shift+s"],
      enabled: () => !!useRepo.getState().snapshot?.changes.length,
      run: () => useRepo.getState().setStashDialog(true),
    },
    {
      id: "git.fetch",
      keywords: "获取 抓取",
      title: "Fetch",
      enabled: () => !!useRepo.getState().root,
      run: () => void useRepo.getState().runRemote("Fetch", (r, id) => api.fetch(r, id), "Fetch 完成"),
    },
    {
      id: "git.pull",
      keywords: "拉取 更新项目 update",
      title: "Update Project (Pull)",
      keys: ["mod+t"],
      enabled: () => !!useRepo.getState().root,
      run: () => {
        const st = useRepo.getState();
        void st.runRemote("Pull", (r, id) => api.pull(r, st.pullMode, id), "已更新到最新");
      },
    },
    {
      id: "git.push",
      keywords: "推送",
      title: "Push…",
      keys: ["mod+shift+k"],
      enabled: () => !!useRepo.getState().root,
      run: () => useRepo.getState().setPushDialog(true),
    },
  ]);

  return (
    <div className="app">
      {root ? (
        <>
          <TabBar />
          <Toolbar />
          <OperationBanner />
          {/* Keyed by repo: per-repo view state (commit draft, scroll) resets on tab switch. */}
          <main className="main" key={root}>
            {view === "changes" ? <ChangesLayout /> : view === "log" ? <LogView /> : <StashView />}
          </main>
        </>
      ) : restored ? (
        <Welcome gitVersion={info?.gitVersion ?? ""} />
      ) : (
        <div className="main" />
      )}
      <StatusBar info={info} />
      <BranchPopup />
      <UndoCenter />
      <CommandPalette />
      <PushDialog />
      <StashDialog />
      <RebaseDialog />
      {annotate && (
        <Suspense fallback={null}>
          <AnnotateView />
        </Suspense>
      )}
      {compare && (
        <Suspense fallback={null}>
          <CompareView />
        </Suspense>
      )}
      {/* After the annotate view: opened from there, it goes on top. */}
      {history && (
        <Suspense fallback={null}>
          <HistoryView />
        </Suspense>
      )}
      {mergePath && (
        <Suspense fallback={null}>
          <MergeTool />
        </Suspense>
      )}
      <ConfirmDialog />
      <AskpassDialog />
      <Toasts />
    </div>
  );
}

function ChangesLayout() {
  const [width, setWidth] = useState(() => load("commitPanelWidth", 380));
  const dragging = useRef(false);

  useEffect(() => {
    const move = (e: MouseEvent) => {
      if (!dragging.current) return;
      setWidth(Math.max(260, Math.min(window.innerWidth - 320, e.clientX)));
    };
    const up = () => {
      if (!dragging.current) return;
      dragging.current = false;
      document.body.classList.remove("resizing");
      setWidth((w) => {
        save("commitPanelWidth", w);
        return w;
      });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);

  return (
    <div className="split">
      <div className="split-left" style={{ width }}>
        <CommitPanel />
      </div>
      <div
        className="splitter"
        onMouseDown={() => {
          dragging.current = true;
          document.body.classList.add("resizing");
        }}
      />
      <div className="split-right">
        <DiffView />
      </div>
    </div>
  );
}
