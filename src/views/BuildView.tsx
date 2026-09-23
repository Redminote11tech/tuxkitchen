import { useState, useEffect } from "react";
import { FolderOpen, Package, Layers, Archive, FileArchive, BadgeCheck } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";

interface BuildViewProps {
  activeProject: Project | null;
}

export function BuildView({ activeProject }: BuildViewProps) {
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [format, setFormat] = useState<string>("ext4");
  const [targetDir, setTargetDir] = useState<string | null>(null);
  const [sparseOut, setSparseOut] = useState(false);
  const [verify, setVerify] = useState(true);
  const [erofsAlgo, setErofsAlgo] = useState<string>("lz4hc");

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
    }
  }, [activeProject]);

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

  const selectTargetDir = async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
      });
      if (dir && typeof dir === "string") {
        setTargetDir(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleBuildImage = async () => {
    if (!targetDir || !workspace) {
      alert("Please select a target directory to build and ensure workspace is set.");
      return;
    }
    const partitionName = targetDir.split('/').pop() || 'partition';
    try {
      await invoke("build_image", {
        inputDir: targetDir,
        outputImg: `${workspace}/${partitionName}_new.img`,
        format,
        sparse: format === "ext4" ? sparseOut : false,
        verify,
        erofsAlgo: format === "erofs" ? erofsAlgo : null,
      });
    } catch (e) {
      console.error(e);
      alert(`Build failed: ${e}`);
    }
  };

  const handleBuildSuper = async () => {
    if (!workspace) return;
    try {
      await invoke("build_super", { workspacePath: workspace, outputImg: `${workspace}/super_new.img` });
    } catch (e) {
      console.error(e);
      alert(`Super build failed: ${e}`);
    }
  };

  const handleBuildTar = async (md5: boolean) => {
    if (!workspace) return;
    try {
      if (md5) {
        await invoke("build_tar_md5", { inputDir: workspace, outputTar: `${workspace}/Odin_Flashable.tar.md5` });
      } else {
        await invoke("build_tar", { inputDir: workspace, outputTar: `${workspace}/Odin_Flashable.tar` });
      }
    } catch (e) {
      console.error(e);
      alert(`Tar build failed: ${e}`);
    }
  };

  const handleCompressLz4 = async () => {
    if (!workspace) return;
    try {
      const file = await open({ multiple: false });
      if (file && typeof file === "string") {
        await invoke("compress_lz4", { input: file, output: `${file}.lz4` });
      }
    } catch (e) {
      console.error(e);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Building & Repacking</div>

        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Project Workspace Directory (Output)</label>
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
        <div className="md-card-title">Build Individual Partition</div>
        <div className="flex-col mt-4">
            <div>
                <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Target Folder to Pack (e.g., system/, vendor/)</label>
                <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
                <input type="text" readOnly value={targetDir || "Select a folder to pack into an image"} />
                <button className="icon-btn" onClick={selectTargetDir} title="Browse Directory">
                    <FolderOpen size={20} />
                </button>
                </div>
            </div>

            <div className="flex-row" style={{ marginTop: '16px', gap: '16px', flexWrap: 'wrap' }}>
                <div className="flex-col" style={{ gap: '8px', flexGrow: 1 }}>
                    <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Format</label>
                    <select
                        value={format}
                        onChange={(e) => setFormat(e.target.value)}
                    >
                        <option value="ext4">Ext4</option>
                        <option value="erofs">EroFS</option>
                        <option value="f2fs">F2FS</option>
                    </select>
                </div>
                {format === "erofs" && (
                    <div className="flex-col" style={{ gap: '8px', flexGrow: 1 }}>
                        <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>EROFS Compression</label>
                        <select value={erofsAlgo} onChange={(e) => setErofsAlgo(e.target.value)}>
                            <option value="lz4hc">LZ4HC (smaller)</option>
                            <option value="lz4">LZ4 (faster)</option>
                            <option value="none">None</option>
                        </select>
                    </div>
                )}
                {format === "ext4" && (
                    <div className="flex-col" style={{ gap: '8px' }}>
                        <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Output</label>
                        <div className="flex-row" style={{ gap: '16px', alignItems: 'center' }}>
                            <label className="flex-row" style={{ gap: '6px', alignItems: 'center', fontSize: '14px' }}>
                                <input type="checkbox" checked={sparseOut} onChange={(e) => setSparseOut(e.target.checked)} />
                                Sparse
                            </label>
                        </div>
                    </div>
                )}
                <div className="flex-col" style={{ gap: '8px' }}>
                    <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Check</label>
                    <label className="flex-row" style={{ gap: '6px', alignItems: 'center', fontSize: '14px' }}>
                        <input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)} />
                        Verify after build
                    </label>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-end', paddingBottom: '2px' }}>
                    <button className="primary flex-row" onClick={handleBuildImage} style={{ display: 'flex', alignItems: 'center', gap: '8px', height: '45px' }}>
                        <Package size={18} />
                        Build Partition Image
                    </button>
                </div>
            </div>
            <p style={{ marginTop: '14px', fontSize: '13px', color: 'var(--md-sys-color-outline)' }}>
              <em>Ownership is set to root:root and SELinux contexts are applied from the ROM's file_contexts (e2fsdroid / sload.f2fs), with a metadata coverage report in the log.</em>
            </p>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">Advanced Building</div>
        <div className="flex-row" style={{ flexWrap: 'wrap', gap: '16px' }}>
          <button className="primary flex-row" onClick={handleBuildSuper} style={{ display: 'flex', alignItems: 'center', gap: '8px', backgroundColor: '#6750A4' }}>
            <Layers size={18} />
            Build super.img
          </button>
          <button className="secondary flex-row" onClick={() => handleBuildTar(false)} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Archive size={18} />
            Create Odin .tar
          </button>
          <button className="secondary flex-row" onClick={() => handleBuildTar(true)} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <BadgeCheck size={18} />
            Create Odin .tar.md5
          </button>
          <button className="secondary flex-row" onClick={handleCompressLz4} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <FileArchive size={18} />
            Compress to .lz4
          </button>
        </div>
      </div>
    </div>
  );
}
