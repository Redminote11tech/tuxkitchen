import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Scale, Play } from "lucide-react";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface DiffEntry {
  path: string;
  status: "identical" | "added" | "removed" | "changed" | "type-changed";
  left_size: number | null;
  right_size: number | null;
  is_dir: boolean;
}

interface CompareSummary {
  identical: number;
  added: number;
  removed: number;
  changed: number;
  type_changed: number;
  truncated: boolean;
}

interface CompareResult {
  summary: CompareSummary;
  entries: DiffEntry[];
}


function fmtSize(bytes: number | null): string {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

interface CompareViewProps {
  activeProject: Project | null;
}

export function CompareView({ activeProject }: CompareViewProps) {
  const [left, setLeft] = useState<string>("");
  const [right, setRight] = useState<string>("");
  const [result, setResult] = useState<CompareResult | null>(null);
  const [statusFilter, setStatusFilter] = useState<string>("differences");
  const [pathFilter, setPathFilter] = useState("");

  useEffect(() => {
    if (activeProject && !left) {
      setLeft(activeProject.path);
    }
  }, [activeProject]);

  const pick = async (side: "left" | "right") => {
    try {
      const dir = await open({ directory: true, multiple: false });
      if (dir && typeof dir === "string") {
        if (side === "left") setLeft(dir);
        else setRight(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const run = async () => {
    if (!left || !right) {
      toast.error("Pick both sides to compare.");
      return;
    }
    try {
      const res = await runBusy("Comparing trees", () =>
        invoke<CompareResult>("compare_trees", { left, right }),
      );
      setResult(res);
      const s = res.summary;
      if (s.changed + s.added + s.removed + s.type_changed === 0) {
        toast.success("Trees are identical.");
      } else {
        toast.info(`${s.changed} changed, ${s.added} added, ${s.removed} removed.`);
      }
    } catch (e) {
      console.error(e);
      toast.error(`Compare failed: ${e}`);
    }
  };

  const visible = (result?.entries ?? []).filter((e) => {
    if (statusFilter === "differences" && e.status === "identical") return false;
    if (statusFilter !== "differences" && statusFilter !== "all" && e.status !== statusFilter) return false;
    const f = pathFilter.trim().toLowerCase();
    return !f || e.path.toLowerCase().includes(f);
  });

  const chip = (label: string, count: number, active: boolean, onClick: () => void) => (
    <button
      key={label}
      onClick={onClick}
      className={`chip ${active ? "chip-ok" : ""}`}
      style={{
        cursor: "pointer",
        border: "1px solid var(--md-sys-color-outline)",
        background: active ? "var(--md-sys-color-primary-container)" : "transparent",
        color: active ? "var(--md-sys-color-on-primary-container)" : "var(--md-sys-color-on-surface-variant)",
      }}
    >
      {label}: {count}
    </button>
  );

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Compare Trees</div>
        <div className="flex-col mt-4">
          {(["left", "right"] as const).map((side) => (
            <div key={side}>
              <label style={{ color: "var(--md-sys-color-outline)", fontSize: 14 }}>
                {side === "left" ? "Left (e.g. stock extract)" : "Right (e.g. your build, or another firmware)"}
              </label>
              <div className="flex-row mt-4" style={{ marginTop: 8 }}>
                <input type="text" readOnly value={side === "left" ? left : right || "No directory selected"} />
                <button className="icon-btn" onClick={() => pick(side)} title="Browse directory">
                  <FolderOpen size={20} />
                </button>
              </div>
            </div>
          ))}
        </div>
        <div style={{ marginTop: 16 }}>
          <button className="primary flex-row" onClick={run} style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <Play size={16} />
            Compare
          </button>
        </div>
        <p style={{ marginTop: 12, fontSize: 13, color: "var(--md-sys-color-outline)" }}>
          <em>Read-only. Same-size files are compared byte for byte. Compare a stock partition tree against your edits, or two firmware releases against each other.</em>
        </p>
      </div>

      {result && (
        <>
          <div className="md-card">
            <div className="flex-row" style={{ flexWrap: "wrap", gap: 10, alignItems: "center" }}>
              <Scale size={18} color="var(--md-sys-color-primary)" />
              {chip("differences", result.summary.added + result.summary.removed + result.summary.changed + result.summary.type_changed, statusFilter === "differences", () => setStatusFilter("differences"))}
              {chip("added", result.summary.added, statusFilter === "added", () => setStatusFilter("added"))}
              {chip("removed", result.summary.removed, statusFilter === "removed", () => setStatusFilter("removed"))}
              {chip("changed", result.summary.changed, statusFilter === "changed", () => setStatusFilter("changed"))}
              {chip("type-changed", result.summary.type_changed, statusFilter === "type-changed", () => setStatusFilter("type-changed"))}
              {chip("identical", result.summary.identical, statusFilter === "identical", () => setStatusFilter("identical"))}
              <input
                placeholder="filter paths..."
                value={pathFilter}
                onChange={(e) => setPathFilter(e.target.value)}
                style={{ marginLeft: "auto", maxWidth: 240 }}
              />
            </div>
            {result.summary.truncated && (
              <p style={{ marginTop: 10, fontSize: 13, color: "var(--md-sys-color-error)" }}>
                Result list truncated at 50,000 entries — the summary counts everything.
              </p>
            )}
          </div>

          <div className="md-card">
            <div className="project-list flex-col" style={{ maxHeight: "52vh", overflowY: "auto" }}>
              {visible.map((e) => (
                <div key={e.path} className="flex-row" style={{
                  padding: "8px 14px",
                  backgroundColor: "var(--md-sys-color-surface-variant)",
                  borderRadius: 8,
                  marginBottom: 6,
                  justifyContent: "space-between",
                }}>
                  <div className="flex-row" style={{ gap: 10, minWidth: 0 }}>
                    <span className={`chip ${e.status === "identical" ? "" : "chip-ok"}`}
                      style={e.status === "identical" ? { opacity: 0.5 } : undefined}>
                      {e.status}
                    </span>
                    <span style={{ fontSize: 13, fontFamily: "monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {e.path}{e.is_dir ? "/" : ""}
                    </span>
                  </div>
                  <span style={{ fontSize: 12, color: "var(--md-sys-color-outline)", whiteSpace: "nowrap" }}>
                    {fmtSize(e.left_size)} → {fmtSize(e.right_size)}
                  </span>
                </div>
              ))}
              {visible.length === 0 && (
                <p style={{ color: "var(--md-sys-color-outline)" }}>
                  Nothing matches this filter.
                </p>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
