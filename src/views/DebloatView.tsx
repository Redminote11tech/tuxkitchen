import { useState, useEffect } from "react";
import { FolderOpen, FileCode, Trash2, Settings2, FileTerminal, ShieldCheck, Undo2 } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface DebloatViewProps {
  activeProject: Project | null;
}

interface AppInfo {
  name: string;
  path: string;
  size: number;
}

interface RemovedApp {
  backup_path: string;
  original_path: string;
  name: string;
}

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

export function DebloatView({ activeProject }: DebloatViewProps) {
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [removed, setRemoved] = useState<RemovedApp[]>([]);
  const [loading, setLoading] = useState(false);
  const [customScript, setCustomScript] = useState<string | null>(null);

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
      loadApps(activeProject.path);
    }
  }, [activeProject]);

  const loadApps = async (dir: string) => {
    setLoading(true);
    try {
      const appList: AppInfo[] = await invoke("list_apps", { workspacePath: dir });
      setApps(appList);
      const removedList: RemovedApp[] = await invoke("list_removed", { workspacePath: dir });
      setRemoved(removedList);
    } catch (e) {
      console.error(e);
    }
    setLoading(false);
  };

  const selectWorkspace = async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
      });
      if (dir && typeof dir === "string") {
        setWorkspace(dir);
        loadApps(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const selectScript = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Bash Script", extensions: ["sh", "bash"] }],
      });
      if (file && typeof file === "string") {
        setCustomScript(file);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleRemoveApp = async (appPath: string) => {
    if (!confirm("Remove this app? It moves into the project backup and can be restored.")) return;
    if (!workspace) return;
    try {
      await invoke("remove_app", { appPath, workspacePath: workspace });
      loadApps(workspace);
    } catch (e) {
      console.error(e);
      toast.error(`Failed to remove app: ${e}`);
    }
  };

  const handleRestoreApp = async (backupPath: string) => {
    if (!workspace) return;
    try {
      await invoke("restore_app", { backupPath });
      loadApps(workspace);
    } catch (e) {
      console.error(e);
      toast.error(`Failed to restore app: ${e}`);
    }
  };

  const handleRunScript = async () => {
    if (!customScript || !workspace) {
      toast.error("Select a script and workspace first.");
      return;
    }
    try {
      await invoke("run_custom_script", { scriptPath: customScript, workspacePath: workspace });
    } catch (e) {
      console.error(e);
    }
  };

  const handleSamsungDisarm = async () => {
    if (!workspace) return;
    try {
      await runBusy("Samsung disarm", () =>
        invoke("run_samsung_disarm", { workspacePath: workspace }),
      );
      toast.success("Disarm finished - check the log for per-image results.");
    } catch (e) {
      console.error(e);
      toast.error(`Disarm failed: ${e}`);
    }
  };

  const handleDeodex = async () => {
    if (!workspace) return;
    try {
      await runBusy("Deodexing apps", () =>
        invoke("run_deodex", { workspacePath: workspace }),
      );
      toast.success("Deodex finished - check the log for per-app results.");
    } catch (e) {
      console.error(e);
      toast.error(`Deodex failed: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">ROM Customization & Debloat</div>

        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Project Workspace Directory</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={workspace || "No workspace selected"} />
              <button className="icon-btn" onClick={selectWorkspace} title="Browse Output">
                <FolderOpen size={20} />
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">Custom Scripts & Mods</div>
        <div className="flex-col mt-4">
            <div className="flex-row" style={{ flexWrap: 'wrap', gap: '16px' }}>
                <button className="secondary flex-row" onClick={handleDeodex} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <Settings2 size={18} />
                    Deodex Framework & Apps
                </button>
                <button className="secondary flex-row" onClick={handleSamsungDisarm} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <ShieldCheck size={18} />
                    Samsung ROM Disarm (Knox/RMM)
                </button>
            </div>
            <div className="mt-4" style={{borderTop: '1px solid var(--md-sys-color-surface-variant)', paddingTop: '16px'}}>
                <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Run Custom Bash Script</label>
                <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
                <input type="text" readOnly value={customScript || "Select a .sh script"} />
                <button className="icon-btn" onClick={selectScript} title="Browse Script">
                    <FileCode size={20} />
                </button>
                <button className="primary flex-row" onClick={handleRunScript} style={{ display: 'flex', alignItems: 'center', gap: '8px', marginLeft: '8px' }}>
                    <FileTerminal size={18} />
                    Run
                </button>
                </div>
            </div>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">One-Click Debloater</div>
        <div className="flex-col mt-4">
            <button className="secondary" onClick={() => workspace && loadApps(workspace)} style={{ width: 'fit-content' }}>
                Refresh App List
            </button>
            {loading ? (
                <p style={{ color: "var(--md-sys-color-outline)", marginTop: '16px' }}>Scanning firmware directories...</p>
            ) : apps.length === 0 ? (
                <p style={{ color: "var(--md-sys-color-outline)", marginTop: '16px' }}>No apps found. Ensure you have unpacked system/product/vendor images.</p>
            ) : (
                <div className="project-list flex-col" style={{ marginTop: '16px', maxHeight: '400px', overflowY: 'auto' }}>
                    {apps.map((app, idx) => (
                    <div key={idx} className="project-item flex-row" style={{
                        padding: '12px 16px',
                        backgroundColor: 'var(--md-sys-color-surface-variant)',
                        borderRadius: '8px',
                        justifyContent: 'space-between',
                        marginBottom: '8px'
                    }}>
                        <div className="flex-col" style={{ gap: '4px' }}>
                            <div style={{ fontWeight: 500, fontSize: '14px' }}>{app.name}</div>
                            <div style={{ fontSize: '11px', color: 'var(--md-sys-color-outline)' }}>{app.path}</div>
                        </div>
                        <div className="flex-row" style={{ gap: '8px', alignItems: 'center' }}>
                            <span style={{ fontSize: '12px', color: 'var(--md-sys-color-outline)' }}>{fmtSize(app.size)}</span>
                            <button className="icon-btn" onClick={() => handleRemoveApp(app.path)} title="Remove App (restorable)">
                                <Trash2 size={18} color="var(--md-sys-color-error)" />
                            </button>
                        </div>
                    </div>
                    ))}
                </div>
            )}
        </div>
      </div>

      {removed.length > 0 && (
        <div className="md-card">
          <div className="md-card-title">Removed Apps ({removed.length}) - restorable</div>
          <div className="project-list flex-col" style={{ marginTop: '12px', maxHeight: '300px', overflowY: 'auto' }}>
            {removed.map((r) => (
              <div key={r.backup_path} className="project-item flex-row" style={{
                padding: '12px 16px',
                backgroundColor: 'var(--md-sys-color-surface-variant)',
                borderRadius: '8px',
                justifyContent: 'space-between',
                marginBottom: '8px'
              }}>
                <div className="flex-col" style={{ gap: '4px' }}>
                  <div style={{ fontWeight: 500, fontSize: '14px' }}>{r.name}</div>
                  <div style={{ fontSize: '11px', color: 'var(--md-sys-color-outline)' }}>restores to {r.original_path}</div>
                </div>
                <button className="icon-btn" onClick={() => handleRestoreApp(r.backup_path)} title="Restore App">
                  <Undo2 size={18} color="var(--md-sys-color-primary)" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
