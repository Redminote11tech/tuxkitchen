import { useState, useEffect } from "react";
import { FolderOpen, FileArchive, ScanSearch } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";

interface ProbeResult {
  kind: string;
  detail: string;
}

interface UnpackerViewProps {
  activeProject: Project | null;
}

export function UnpackerView({ activeProject }: UnpackerViewProps) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
    }
  }, [activeProject]);

  const selectFile = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "ROM", extensions: ["zip", "tar", "md5", "7z", "lz4", "tgz", "gz", "xz", "zst", "br", "bz2"] }],
      });
      if (file && typeof file === "string") {
        setSelectedFile(file);
        setProbe(null);
        try {
          setProbe(await invoke<ProbeResult>("probe_file", { path: file }));
        } catch (e) {
          console.error(e);
        }
      }
    } catch (e) {
      console.error(e);
    }
  };

  const selectWorkspace = async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
      });
      if (dir && typeof dir === "string") {
        setWorkspace(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleUnpack = async () => {
    if (!selectedFile || !workspace) {
      alert("Please select both a file and a workspace.");
      return;
    }
    try {
      await invoke("unpack_rom", { filePath: selectedFile, workspacePath: workspace });
    } catch (e) {
      console.error(e);
    }
  };

  const handleBuildSuper = async () => {
    if (!workspace) {
      alert("Please select a workspace containing the partition images.");
      return;
    }
    try {
      await invoke("build_super", { workspacePath: workspace, outputImg: `${workspace}/super_new.img` });
    } catch (e) {
      console.error(e);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Project Setup</div>

        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Firmware File (.zip, .tar.md5, .tar, .7z, .lz4, .br)</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={selectedFile || "No file selected"} />
              <button className="icon-btn" onClick={selectFile} title="Browse File">
                <FileArchive size={20} />
              </button>
            </div>
          </div>

          <div className="mt-4">
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Workspace Directory</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={workspace || "No workspace selected"} />
              <button className="icon-btn" onClick={selectWorkspace} title="Browse Workspace">
                <FolderOpen size={20} />
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">Actions</div>
        <div className="flex-row">
          <button className="primary" onClick={handleUnpack}>Extract & Unpack ROM</button>
          <button className="secondary" onClick={handleBuildSuper}>Build super.img</button>
        </div>
      </div>

      {probe && (
        <div className="md-card probe-card">
          <div className="flex-row" style={{ gap: "10px", alignItems: "center" }}>
            <ScanSearch size={18} color="var(--md-sys-color-primary)" />
            <span className={`chip ${probe.kind === "unknown" ? "chip-warn" : "chip-ok"}`}>{probe.kind}</span>
          </div>
          <p style={{ marginTop: "10px", fontSize: "13px", color: "var(--md-sys-color-outline)" }}>{probe.detail}</p>
        </div>
      )}
    </div>
  );
}
