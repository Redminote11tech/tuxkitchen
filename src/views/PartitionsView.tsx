import { useState, useEffect } from "react";
import { FolderOpen, FileCode, Cpu, Layers, ArchiveRestore, HardDrive, FileText, Component, PackageOpen, Database } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";

interface ProbeResult {
  kind: string;
  detail: string;
}

interface PartitionsViewProps {
  activeProject: Project | null;
}

export function PartitionsView({ activeProject }: PartitionsViewProps) {
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
        filters: [{ name: "Image/Archive/Bin", extensions: ["img", "dat", "br", "lz4", "ext4", "f2fs", "erofs", "bin", "payload"] }],
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

  const handleAction = async (action: string) => {
    if (!selectedFile || !workspace) {
      alert("Please select both a file and a workspace/output directory.");
      return;
    }
    try {
      if (action === "sparse") {
        await invoke("convert_sparse", { input: selectedFile, output: `${workspace}/raw.img` });
      } else if (action === "to_sparse") {
        await invoke("to_sparse", { input: selectedFile, output: `${workspace}/sparse.img` });
      } else if (action === "super") {
        await invoke("unpack_super", { input: selectedFile, outputDir: workspace });
      } else if (action === "brotli") {
        await invoke("decompress_brotli", { input: selectedFile, output: selectedFile.replace('.br', '') });
      } else if (action === "payload") {
        await invoke("extract_payload", { payloadPath: selectedFile, outputDir: workspace });
      } else if (action === "ext4") {
        await invoke("extract_ext4", { input: selectedFile, outputDir: `${workspace}/extracted_ext4` });
      } else if (action === "erofs") {
        await invoke("extract_erofs", { input: selectedFile, outputDir: `${workspace}/extracted_erofs` });
      } else if (action === "f2fs") {
        await invoke("extract_f2fs", { input: selectedFile, outputDir: `${workspace}/extracted_f2fs` });
      } else if (action === "file_contexts") {
        await invoke("convert_file_contexts", { input: selectedFile, output: `${workspace}/file_contexts.txt` });
      }
    } catch (e) {
      console.error(e);
      alert(`Action failed: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Partition Management</div>

        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Source File (.img, .dat.br, .payload.bin, .ext4)</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={selectedFile || "No file selected"} placeholder="Select super.img, system.img, payload.bin, etc..." />
              <button className="icon-btn" onClick={selectFile} title="Browse File">
                <FileCode size={20} />
              </button>
            </div>
          </div>

          <div className="mt-4">
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Output Directory</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={workspace || "No workspace selected"} />
              <button className="icon-btn" onClick={selectWorkspace} title="Browse Output">
                <FolderOpen size={20} />
              </button>
            </div>
          </div>
        </div>
      </div>

      {probe && (
        <div className="md-card probe-card">
          <div className="flex-row" style={{ gap: "10px", alignItems: "center" }}>
            <span className={`chip ${probe.kind === "unknown" ? "chip-warn" : "chip-ok"}`}>{probe.kind}</span>
            <span style={{ fontSize: "13px", color: "var(--md-sys-color-outline)" }}>{probe.detail}</span>
          </div>
        </div>
      )}

      <div className="md-card">
        <div className="md-card-title">Operations</div>
        <div className="flex-row" style={{ flexWrap: 'wrap', gap: '16px' }}>
          <button className="primary flex-row" onClick={() => handleAction("super")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Layers size={18} />
            Unpack super.img
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("payload")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <PackageOpen size={18} />
            Extract payload.bin
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("sparse")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Cpu size={18} />
            Sparse to Raw
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("to_sparse")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Database size={18} />
            Raw to Sparse
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("brotli")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <ArchiveRestore size={18} />
            Decompress .br
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("ext4")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <HardDrive size={18} />
            Extract Ext4
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("erofs")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Component size={18} />
            Extract EroFS
          </button>
          <button className="secondary flex-row" disabled title="F2FS extraction needs a root mount; building F2FS is supported" style={{ display: 'flex', alignItems: 'center', gap: '8px', opacity: 0.5, cursor: 'not-allowed' }}>
            <HardDrive size={18} />
            Extract F2FS
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("file_contexts")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <FileText size={18} />
            Convert file_contexts.bin
          </button>
        </div>
        <p style={{ marginTop: '14px', fontSize: '13px', color: 'var(--md-sys-color-outline)' }}>
          <em>Full OTA payloads (ZERO/REPLACE blobs) extract directly. Delta payloads (SOURCE_COPY/PUFFDIFF) need the old image and are reported instead of guessed.</em>
        </p>
      </div>
    </div>
  );
}
