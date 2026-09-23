import { useState, useEffect } from "react";
import { FolderOpen, FileCode, ShieldCheck, Wrench, Package, ArrowDownToLine, Component } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface MagiskViewProps {
  activeProject: Project | null;
}

export function MagiskView({ activeProject }: MagiskViewProps) {
  const [selectedBoot, setSelectedBoot] = useState<string | null>(null);
  const [selectedApk, setSelectedApk] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<string | null>(null);

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
    }
  }, [activeProject]);

  const selectBoot = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Boot/VBMeta/DTBO Image", extensions: ["img"] }],
      });
      if (file && typeof file === "string") {
        setSelectedBoot(file);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const selectApk = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Magisk APK", extensions: ["apk", "zip"] }],
      });
      if (file && typeof file === "string") {
        setSelectedApk(file);
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
    if (!selectedBoot || !workspace) {
      toast.error("Please select an image and an output directory.");
      return;
    }
    try {
      await runBusy(`Boot Lab: ${action}`, async () => {
        if (action === "unpack") {
          await invoke("unpack_boot", { input: selectedBoot, outputDir: workspace });
        } else if (action === "repack") {
          await invoke("repack_boot", { inputDir: workspace, output: `${workspace}/new-boot.img` });
        } else if (action === "patch") {
          if (!selectedApk) {
              toast.error("Please select a Magisk APK to patch.");
              return;
          }
          await invoke("patch_magisk", { bootImage: selectedBoot, magiskApk: selectedApk, outputDir: workspace });
        } else if (action === "vbmeta") {
          await invoke("patch_vbmeta", { vbmetaImage: selectedBoot, outputDir: workspace });
        } else if (action === "dtbo_unpack") {
          await invoke("dtbo_unpack", { input: selectedBoot, outputDir: `${workspace}/dtbo_parts` });
        } else if (action === "dtbo_pack") {
          await invoke("dtbo_pack", { inputDir: `${workspace}/dtbo_parts`, output: `${workspace}/dtbo_new.img`, pageSize: null });
        }
      });
    } catch (e) {
      console.error(e);
      toast.error(`Operation failed: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Boot & Recovery Tools</div>

        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Boot/VBMeta/DTBO Image (.img)</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={selectedBoot || "No image selected"} placeholder="Select boot.img, recovery.img, vbmeta.img, dtbo.img..." />
              <button className="icon-btn" onClick={selectBoot} title="Browse File">
                <FileCode size={20} />
              </button>
            </div>
          </div>

          <div className="mt-4">
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Magisk APK (Required for patching)</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={selectedApk || "No APK selected"} placeholder="Select Magisk v27+.apk" />
              <button className="icon-btn" onClick={selectApk} title="Browse File">
                <Package size={20} />
              </button>
            </div>
          </div>

          <div className="mt-4">
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Workspace Directory</label>
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
        <div className="md-card-title">Operations</div>
        <div className="flex-row" style={{ flexWrap: 'wrap', gap: '16px' }}>
          <button className="primary flex-row" onClick={() => handleAction("unpack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <ArrowDownToLine size={18} />
            Unpack Boot
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("repack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Package size={18} />
            Repack Boot
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("patch")} style={{ display: 'flex', alignItems: 'center', gap: '8px', backgroundColor: '#006B5B', color: 'white' }}>
            <ShieldCheck size={18} />
            Patch Magisk
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("vbmeta")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Wrench size={18} />
            Patch VBMeta
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("dtbo_unpack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Component size={18} />
            Unpack DTBO
          </button>
          <button className="secondary flex-row" onClick={() => handleAction("dtbo_pack")} style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Component size={18} />
            Pack DTBO
          </button>
        </div>
        <p style={{ marginTop: '14px', fontSize: '13px', color: 'var(--md-sys-color-outline)' }}>
          <em>Patch VBMeta writes flags 3 (HASHTREE_DISABLED | VERIFICATION_DISABLED). DTBO operations work on the <code>dtbo_parts/</code> folder in the workspace.</em>
        </p>
      </div>
    </div>
  );
}
