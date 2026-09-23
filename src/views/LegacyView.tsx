import { useState, useEffect } from "react";
import { FolderOpen, FileCode, ArrowDownToLine, Package, AlertTriangle } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";

interface LegacyViewProps {
  activeProject: Project | null;
}

export function LegacyView({ activeProject }: LegacyViewProps) {
  const [aikPath, setAikPath] = useState<string | null>(null);
  const [selectedBoot, setSelectedBoot] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<string | null>(null);

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
    }
  }, [activeProject]);

  const selectAikPath = async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
      });
      if (dir && typeof dir === "string") {
        setAikPath(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const selectBoot = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Boot Image", extensions: ["img"] }],
      });
      if (file && typeof file === "string") {
        setSelectedBoot(file);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleAction = async (action: string) => {
    if (!aikPath) {
        toast.error("Please set the path to your Android Image Kitchen directory.");
        return;
    }
    try {
      if (action === "unpack") {
        if (!selectedBoot) { toast.error("Please select a boot image to unpack."); return; }
        await invoke("aik_unpack", { aikPath, bootImage: selectedBoot });
      } else if (action === "repack") {
        if (!workspace) { toast.error("Please ensure a workspace output directory is set."); return; }
        await invoke("aik_repack", { aikPath, outputImage: `${workspace}/image-new.img` });
      }
    } catch (e) {
      console.error(e);
      toast.error(`AIK Error: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card" style={{ backgroundColor: 'rgba(242, 184, 181, 0.1)', border: '1px solid var(--md-sys-color-error)' }}>
        <div className="flex-row" style={{ color: 'var(--md-sys-color-error)', fontWeight: 500 }}>
            <AlertTriangle size={20} />
            Nostalgia / Legacy Tools Warning
        </div>
        <p style={{ marginTop: '12px', fontSize: '14px', lineHeight: '1.5' }}>
            This module integrates osm0sis's <strong>Android Image Kitchen (AIK)</strong>. It is designed for legacy Android devices (Android 7.0 - 11.0). 
            For modern Android 12+ devices using vendor_boot or GKI formats, please use the standard `magiskboot` tools located in the <strong>Magisk</strong> tab.
        </p>
      </div>

      <div className="md-card">
        <div className="md-card-title">AIK Configuration</div>
        
        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Local AIK Directory (Folder containing unpackimg.sh)</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={aikPath || "No AIK folder selected"} placeholder="Select AIK installation directory" />
              <button className="icon-btn" onClick={selectAikPath} title="Browse Directory">
                <FolderOpen size={20} />
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">Legacy Boot Operations</div>
        
        <div className="flex-col mt-4">
            <div>
                <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Boot Image to Unpack</label>
                <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
                <input type="text" readOnly value={selectedBoot || "No boot image selected"} />
                <button className="icon-btn" onClick={selectBoot} title="Browse File">
                    <FileCode size={20} />
                </button>
                </div>
            </div>

            <div className="flex-row" style={{ flexWrap: 'wrap', gap: '16px', marginTop: '24px' }}>
                <button className="primary flex-row" onClick={() => handleAction("unpack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <ArrowDownToLine size={18} />
                    Unpack using AIK
                </button>
                <button className="secondary flex-row" onClick={() => handleAction("repack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <Package size={18} />
                    Repack and Export Image
                </button>
            </div>
            
            <p style={{ marginTop: '16px', fontSize: '13px', color: 'var(--md-sys-color-outline)' }}>
                <em>Note: AIK unpacks files into its own 'split_img' and 'ramdisk' subfolders. Clicking "Repack" will combine whatever is currently in those folders and copy the new image to your active workspace.</em>
            </p>
        </div>
      </div>
    </div>
  );
}