import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  Folder, FolderOpen, FileCode, ChevronRight, CornerLeftUp,
  Pencil, Trash2, ScanSearch, Hammer, BadgeCheck,
} from "lucide-react";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface EntryInfo {
  name: string;
  is_dir: boolean;
  size: number;
}

interface ProbeResult {
  kind: string;
  detail: string;
}

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

interface FilesViewProps {
  activeProject: Project | null;
}

export function FilesView({ activeProject }: FilesViewProps) {
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [rel, setRel] = useState("");
  const [entries, setEntries] = useState<EntryInfo[]>([]);
  const [selected, setSelected] = useState<EntryInfo | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
      setRel("");
    }
  }, [activeProject]);

  const load = async (ws: string, path: string) => {
    try {
      const list = await invoke<EntryInfo[]>("list_dir", { workspacePath: ws, rel: path });
      setEntries(list);
      setSelected(null);
      setProbe(null);
    } catch (e) {
      toast.error(`Could not list directory: ${e}`);
    }
  };

  useEffect(() => {
    if (workspace) load(workspace, rel);
  }, [workspace, rel]);

  const navigate = (name: string) => {
    setRel((r) => (r ? `${r}/${name}` : name));
  };

  const up = () => {
    setRel((r) => r.split("/").slice(0, -1).join("/"));
  };

  const selectFile = async (entry: EntryInfo) => {
    setSelected(entry);
    if (!workspace) return;
    const full = rel ? `${rel}/${entry.name}` : entry.name;
    try {
      setProbe(await invoke<ProbeResult>("probe_file", { path: `${workspace}/${full}` }));
    } catch (e) {
      console.error(e);
      setProbe(null);
    }
  };

  const renameSelected = async () => {
    if (!selected || !workspace) return;
    const newName = prompt("New name:", selected.name);
    if (!newName || newName === selected.name) return;
    const full = rel ? `${rel}/${selected.name}` : selected.name;
    try {
      await runBusy("Renaming", () =>
        invoke("rename_path", { workspacePath: workspace, rel: full, newName }),
      );
      toast.success(`Renamed to ${newName}`);
      load(workspace, rel);
    } catch (e) {
      toast.error(`Rename failed: ${e}`);
    }
  };

  const deleteSelected = async () => {
    if (!selected || !workspace) return;
    if (!confirm(`Move ${selected.name} to the project backup? (Restorable in Debloat)`)) return;
    const full = rel ? `${rel}/${selected.name}` : selected.name;
    try {
      await runBusy("Moving to backup", () =>
        invoke("delete_path", { workspacePath: workspace, rel: full }),
      );
      toast.success(`${selected.name} moved to backup (restorable).`);
      load(workspace, rel);
    } catch (e) {
      toast.error(`Delete failed: ${e}`);
    }
  };

  const decompile = async () => {
    if (!selected || !workspace) return;
    const full = rel ? `${rel}/${selected.name}` : selected.name;
    const outDir = `${workspace}/${rel || "."}/${selected.name}_src`;
    try {
      await runBusy(`Decompiling ${selected.name}`, () =>
        invoke("apktool_decompile", { apkPath: `${workspace}/${full}`, outDir }),
      );
      toast.success(`Decompiled into ${selected.name}_src`);
      load(workspace, rel);
    } catch (e) {
      toast.error(`apktool failed: ${e}`);
    }
  };

  const recompileHere = async () => {
    if (!workspace) return;
    const dir = `${workspace}/${rel}`;
    try {
      await runBusy("Recompiling with apktool", () =>
        invoke("apktool_recompile", { apkDir: dir }),
      );
      toast.success("Recompiled — output is in dist/ inside the folder. Select it and use Sign APK to make it installable.");
    } catch (e) {
      toast.error(`apktool failed: ${e}`);
    }
  };

  const signSelected = async () => {
    if (!selected || !workspace) return;
    const full = `${workspace}/${rel ? `${rel}/` : ""}${selected.name}`;
    try {
      await runBusy(`Signing ${selected.name}`, () =>
        invoke("sign_apk", { apkPath: full }),
      );
      toast.success(`${selected.name} signed (v1+v2+v3, debug key) — installable now.`);
    } catch (e) {
      toast.error(`Signing failed: ${e}`);
    }
  };

  const crumbs = rel ? rel.split("/") : [];
  const wsName = (workspace ?? "").split("/").filter(Boolean).pop() || "workspace";
  const hereHasApktool = entries.some((e) => e.name === "apktool.yml");
  const visibleEntries = filter.trim()
    ? entries.filter((e) => e.name.toLowerCase().includes(filter.trim().toLowerCase()))
    : entries;

  const revealWorkspace = async () => {
    if (!workspace) return;
    try {
      await openPath(workspace);
    } catch (e) {
      toast.error(`Could not open file manager: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="flex-row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
          <div className="md-card-title" style={{ margin: 0 }}>Workspace Files</div>
          <div className="flex-row" style={{ gap: 8 }}>
            <input
              placeholder="filter this folder..."
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              style={{ maxWidth: 220 }}
            />
            <button className="secondary" onClick={revealWorkspace} title="Open the workspace in your file manager">
              Reveal
            </button>
          </div>
        </div>
        {!workspace ? (
          <p style={{ color: "var(--md-sys-color-outline)" }}>Open a project first.</p>
        ) : (
          <div className="flex-row" style={{ flexWrap: "wrap", gap: 4, fontSize: 14 }}>
            <button className="secondary" style={{ padding: "6px 14px" }} onClick={() => setRel("")}>
              {wsName}
            </button>
            {crumbs.map((c, i) => (
              <span key={i} className="flex-row" style={{ gap: 4 }}>
                <ChevronRight size={14} color="var(--md-sys-color-outline)" />
                <button
                  className="secondary"
                  style={{ padding: "6px 14px" }}
                  onClick={() => setRel(crumbs.slice(0, i + 1).join("/"))}
                >
                  {c}
                </button>
              </span>
            ))}
            {hereHasApktool && (
              <button className="primary flex-row" style={{ marginLeft: "auto", gap: 8 }} onClick={recompileHere}>
                <Hammer size={16} />
                Recompile (apktool)
              </button>
            )}
          </div>
        )}
      </div>

      {workspace && (
        <div className="md-card">
          <div className="project-list flex-col" style={{ maxHeight: "52vh", overflowY: "auto" }}>
            {rel && (
              <div className="flex-row" style={{ padding: "10px 16px", gap: 12, cursor: "pointer", color: "var(--md-sys-color-outline)" }} onClick={up}>
                <CornerLeftUp size={18} />
                <span style={{ fontSize: 14 }}>.</span>
              </div>
            )}
            {visibleEntries.map((e) => (
              <div
                key={e.name}
                className="flex-row"
                style={{
                  padding: "10px 16px",
                  backgroundColor: "var(--md-sys-color-surface-variant)",
                  borderRadius: 8,
                  marginBottom: 6,
                  justifyContent: "space-between",
                  cursor: "pointer",
                }}
                onClick={() => (e.is_dir ? navigate(e.name) : selectFile(e))}
              >
                <div className="flex-row" style={{ gap: 12 }}>
                  {e.is_dir
                    ? <FolderOpen size={18} color="var(--md-sys-color-primary)" />
                    : <FileCode size={18} color="var(--md-sys-color-outline)" />}
                  <span style={{ fontSize: 14 }}>{e.name}</span>
                </div>
                <span style={{ fontSize: 12, color: "var(--md-sys-color-outline)" }}>
                  {e.is_dir ? "folder" : fmtSize(e.size)}
                </span>
              </div>
            ))}
            {visibleEntries.length === 0 && (
              <p style={{ color: "var(--md-sys-color-outline)" }}>
                {filter ? "Nothing matches the filter." : "Empty directory."}
              </p>
            )}
          </div>
        </div>
      )}

      {selected && (
        <div className="md-card probe-card">
          <div className="flex-row" style={{ flexWrap: "wrap", gap: 12, alignItems: "center" }}>
            <ScanSearch size={18} color="var(--md-sys-color-primary)" />
            <span className={`chip ${probe && probe.kind !== "unknown" ? "chip-ok" : "chip-warn"}`}>
              {probe ? probe.kind : "…"}
            </span>
            <span style={{ fontSize: 13, color: "var(--md-sys-color-outline)", flex: 1 }}>
              {probe ? probe.detail : "Probing..."}
            </span>
            {selected.name.endsWith(".apk") && (
              <button className="primary flex-row" style={{ gap: 8 }} onClick={decompile}>
                <Hammer size={16} />
                Decompile (apktool)
              </button>
            )}
            {selected.name.endsWith(".apk") && (
              <button className="secondary flex-row" style={{ gap: 8 }} onClick={signSelected}>
                <BadgeCheck size={16} />
                Sign APK
              </button>
            )}
            <button className="icon-btn" title="Rename" onClick={renameSelected}>
              <Pencil size={18} />
            </button>
            <button className="icon-btn" title="Move to backup (restorable)" onClick={deleteSelected}>
              <Trash2 size={18} color="var(--md-sys-color-error)" />
            </button>
          </div>
          <p style={{ marginTop: 10, fontSize: 12, color: "var(--md-sys-color-outline)", display: "flex", alignItems: "center", gap: 6 }}>
            <Folder size={12} />
            Deleting moves files into <code>.tuxkitchen/removed/</code> — restore them from the Debloat screen.
          </p>
        </div>
      )}
    </div>
  );
}
