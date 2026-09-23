import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileCog, Plus, Save, Trash2 } from "lucide-react";
import { Project } from "./ProjectsView";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface PropFileInfo {
  rel: string;
  size: number;
}

interface PropRow {
  lineIdx: number;
  key: string;
  value: string;
  deleted: boolean;
}

interface PropsViewProps {
  activeProject: Project | null;
}

function isPropLine(line: string): boolean {
  const t = line.trim();
  return !t.startsWith("#") && t.includes("=");
}

export function PropsView({ activeProject }: PropsViewProps) {
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [files, setFiles] = useState<PropFileInfo[]>([]);
  const [selected, setSelected] = useState<string>("");
  const [raw, setRaw] = useState<string>("");
  const [rows, setRows] = useState<PropRow[]>([]);
  const [added, setAdded] = useState<{ key: string; value: string }[]>([]);
  const [dirty, setDirty] = useState(false);
  const [rawMode, setRawMode] = useState(false);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    if (activeProject) {
      setWorkspace(activeProject.path);
      loadFiles(activeProject.path);
    }
  }, [activeProject]);

  const loadFiles = async (ws: string) => {
    try {
      const list = await invoke<PropFileInfo[]>("list_prop_files", { workspacePath: ws });
      setFiles(list);
      if (list.length > 0) {
        loadFile(ws, list[0].rel);
      } else {
        toast.info("No build.prop found - unpack a ROM first.");
      }
    } catch (e) {
      toast.error(`Could not scan for prop files: ${e}`);
    }
  };

  const loadFile = async (ws: string, rel: string) => {
    setSelected(rel);
    try {
      const content = await invoke<string>("read_prop_file", { workspacePath: ws, rel });
      setRaw(content);
      setRows(parseRows(content));
      setAdded([]);
      setDirty(false);
    } catch (e) {
      toast.error(`Could not read ${rel}: ${e}`);
    }
  };

  const parseRows = (content: string): PropRow[] =>
    content
      .split("\n")
      .map((line, i) => ({ line, i }))
      .filter(({ line }) => isPropLine(line))
      .map(({ line, i }) => {
        const idx = line.indexOf("=");
        return {
          lineIdx: i,
          key: line.slice(0, idx).trim(),
          value: line.slice(idx + 1),
          deleted: false,
        };
      });

  const visibleRows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return rows.filter((r) => !f || r.key.toLowerCase().includes(f) || r.value.toLowerCase().includes(f));
  }, [rows, filter]);

  const updateValue = (lineIdx: number, value: string) => {
    setRows((rs) => rs.map((r) => (r.lineIdx === lineIdx ? { ...r, value, deleted: false } : r)));
    setDirty(true);
  };

  const deleteRow = (lineIdx: number) => {
    setRows((rs) => rs.map((r) => (r.lineIdx === lineIdx ? { ...r, deleted: true } : r)));
    setDirty(true);
  };

  const serialize = (): string => {
    const byLine = new Map(rows.map((r) => [r.lineIdx, r]));
    // Rebuild prop lines with edited values, keep comments/blanks verbatim.
    const rebuilt = raw.split("\n").flatMap((line, i) => {
      const row = byLine.get(i);
      if (!row) return [line];
      if (row.deleted) return [];
      return [`${row.key}=${row.value}`];
    });
    for (const a of added) {
      if (a.key.trim()) rebuilt.push(`${a.key.trim()}=${a.value}`);
    }
    return rebuilt.join("\n");
  };

  const save = async () => {
    if (!workspace || !selected) return;
    const content = rawMode ? raw : serialize();
    try {
      await runBusy(`Saving ${selected}`, () =>
        invoke("save_prop_file", { workspacePath: workspace, rel: selected, content }),
      );
      setRaw(content);
      setRows(parseRows(content));
      setAdded([]);
      setDirty(false);
      toast.success(`${selected} saved (backup written alongside).`);
    } catch (e) {
      toast.error(`Save failed: ${e}`);
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">build.prop Editor</div>
        <div className="flex-row" style={{ flexWrap: "wrap", gap: 16 }}>
          <div className="flex-col" style={{ gap: 6, flexGrow: 1, minWidth: 260 }}>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: 14 }}>
              Prop file {workspace ? `in ${workspace}` : "(select a project first)"}
            </label>
            <select value={selected} onChange={(e) => workspace && loadFile(workspace, e.target.value)}>
              {files.length === 0 && <option>no prop files found</option>}
              {files.map((f) => (
                <option key={f.rel} value={f.rel}>
                  {f.rel} ({f.size} B)
                </option>
              ))}
            </select>
          </div>
          <div className="flex-col" style={{ gap: 6 }}>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: 14 }}>Mode</label>
            <button className={`secondary ${rawMode ? "" : ""}`} onClick={() => setRawMode((m) => !m)}>
              {rawMode ? "Switch to table" : "Switch to raw"}
            </button>
          </div>
          <div style={{ display: "flex", alignItems: "flex-end" }}>
            <button className="primary" onClick={save} disabled={!selected || (!dirty && !rawMode)}>
              <Save size={16} style={{ marginRight: 8, verticalAlign: "middle" }} />
              {dirty ? "Save*" : "Save"}
            </button>
          </div>
        </div>
      </div>

      {rawMode ? (
        <div className="md-card">
          <div className="md-card-title">Raw content — {selected}</div>
          <textarea
            value={raw}
            onChange={(e) => setRaw(e.target.value)}
            spellCheck={false}
            style={{
              width: "100%",
              minHeight: "420px",
              fontFamily: "'Courier New', monospace",
              fontSize: 13,
              background: "var(--md-sys-color-surface-variant)",
              color: "var(--md-sys-color-on-surface)",
              border: "1px solid var(--md-sys-color-outline)",
              borderRadius: 8,
              padding: 12,
            }}
          />
        </div>
      ) : (
        <>
          <div className="md-card">
            <div className="flex-row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
              <div className="md-card-title" style={{ margin: 0 }}>
                Properties — {selected} {dirty && <span style={{ color: "var(--md-sys-color-primary)" }}>(unsaved)</span>}
              </div>
              <input
                placeholder="filter properties..."
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                style={{ maxWidth: 260 }}
              />
            </div>
            <div className="project-list flex-col" style={{ marginTop: 12, maxHeight: "46vh", overflowY: "auto" }}>
              {visibleRows.map((r) => (
                <div
                  key={r.lineIdx}
                  className="flex-row"
                  style={{
                    padding: "8px 12px",
                    backgroundColor: r.deleted ? "rgba(242,184,181,0.08)" : "var(--md-sys-color-surface-variant)",
                    borderRadius: 8,
                    marginBottom: 6,
                    gap: 10,
                    opacity: r.deleted ? 0.5 : 1,
                  }}
                >
                  <span style={{ fontFamily: "monospace", fontSize: 13, minWidth: 220, color: "var(--md-sys-color-primary)" }}>
                    {r.key}
                  </span>
                  <input
                    value={r.value}
                    onChange={(e) => updateValue(r.lineIdx, e.target.value)}
                    style={{ fontFamily: "monospace", fontSize: 13, flex: 1 }}
                  />
                  <button className="icon-btn" title="Remove property" onClick={() => deleteRow(r.lineIdx)}>
                    <Trash2 size={16} color="var(--md-sys-color-error)" />
                  </button>
                </div>
              ))}
              {visibleRows.length === 0 && (
                <p style={{ color: "var(--md-sys-color-outline)" }}>No properties match the filter.</p>
              )}
            </div>
          </div>

          <div className="md-card">
            <div className="md-card-title">Add property</div>
            {added.map((a, i) => (
              <div key={i} className="flex-row" style={{ marginBottom: 8 }}>
                <input
                  placeholder="key (e.g. ro.build.display.id)"
                  value={a.key}
                  onChange={(e) =>
                    setAdded((xs) => xs.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)))
                  }
                  style={{ fontFamily: "monospace", fontSize: 13, flex: 1 }}
                />
                <input
                  placeholder="value"
                  value={a.value}
                  onChange={(e) =>
                    setAdded((xs) => xs.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))
                  }
                  style={{ fontFamily: "monospace", fontSize: 13, flex: 1 }}
                />
                <button className="icon-btn" onClick={() => setAdded((xs) => xs.filter((_, j) => j !== i))}>
                  <Trash2 size={16} />
                </button>
              </div>
            ))}
            <button className="secondary" onClick={() => setAdded((xs) => [...xs, { key: "", value: "" }])}>
              <Plus size={16} style={{ marginRight: 8, verticalAlign: "middle" }} />
              Add row
            </button>
          </div>
        </>
      )}

      <div className="md-card probe-card">
        <div className="flex-row" style={{ gap: 10, alignItems: "center" }}>
          <FileCog size={18} color="var(--md-sys-color-primary)" />
          <span style={{ fontSize: 13, color: "var(--md-sys-color-outline)" }}>
            Comments and blank lines survive every save. A <code>.prop.bak</code> copy of the previous
            version is written next to the file on every save.
          </span>
        </div>
      </div>
    </div>
  );
}
