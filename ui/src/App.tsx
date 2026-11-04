import { useEffect, useRef, useState } from "react";
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
import { PushDialog } from "./views/PushDialog";
import { Welcome } from "./views/Welcome";

export function App() {
  const root = useRepo((s) => s.root);
  const view = useRepo((s) => s.view);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => installKeyHandler(), []);

  useEffect(() => {
    api.appInfo().then(setInfo).catch(() => {});
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
    { id: "repo.open", title: "Open Repository…", keys: ["mod+o"], run: () => void pickRepository() },
    {
      id: "repo.refresh",
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
      title: "Branches…",
      keys: ["mod+shift+backquote", "mod+b"],
      enabled: () => !!useRepo.getState().root,
      run: () => {
        const st = useRepo.getState();
        st.setBranchPopup(!st.branchPopup);
      },
    },
    { id: "view.changes", title: "Commit", keys: ["mod+0"], run: () => useRepo.getState().setView("changes") },
    { id: "view.log", title: "Log", keys: ["mod+9"], run: () => useRepo.getState().setView("log") },
    {
      id: "git.fetch",
      title: "Fetch",
      enabled: () => !!useRepo.getState().root,
      run: () => void useRepo.getState().runRemote("Fetch", (r, id) => api.fetch(r, id), "Fetch 完成"),
    },
    {
      id: "git.pull",
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
          <Toolbar />
          <main className="main">{view === "changes" ? <ChangesLayout /> : <LogView />}</main>
        </>
      ) : (
        <Welcome gitVersion={info?.gitVersion ?? ""} />
      )}
      <StatusBar gitVersion={info?.gitVersion ?? ""} />
      <BranchPopup />
      <PushDialog />
      <ConfirmDialog />
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
