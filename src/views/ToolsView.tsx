import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, XCircle, RefreshCw } from "lucide-react";

interface ToolInfo {
  name: string;
  purpose: string;
  installed: boolean;
  hint: string;
}

export function ToolsView() {
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const load = async () => {
    setLoading(true);
    try {
      setTools(await invoke<ToolInfo[]>("check_tools"));
    } catch (e) {
      console.error(e);
    }
    setLoading(false);
  };

  useEffect(() => {
    load();
  }, []);

  const missing = tools.filter((t) => !t.installed).length;

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="flex-row" style={{ justifyContent: "space-between", alignItems: "center" }}>
          <div className="md-card-title">Environment Tools</div>
          <button className="icon-btn" onClick={load} title="Rescan">
            <RefreshCw size={18} />
          </button>
        </div>
        <p style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>
          TuxKitchen drives your system's ROM tooling - everything runs locally, nothing is bundled.
          {loading
            ? " Scanning PATH..."
            : missing === 0
              ? " All tools found."
              : ` ${missing} optional tool(s) missing - the features below need them.`}
        </p>
      </div>

      <div className="md-card">
        <div className="project-list flex-col" style={{ maxHeight: "60vh", overflowY: "auto" }}>
          {tools.map((tool) => (
            <div key={tool.name} className="project-item flex-row" style={{
              padding: "12px 16px",
              backgroundColor: "var(--md-sys-color-surface-variant)",
              borderRadius: "8px",
              justifyContent: "space-between",
              marginBottom: "8px",
              alignItems: "center",
            }}>
              <div className="flex-row" style={{ gap: "12px", alignItems: "center" }}>
                {tool.installed
                  ? <CheckCircle2 size={20} color="var(--md-sys-color-primary)" />
                  : <XCircle size={20} color="var(--md-sys-color-error)" />}
                <div className="flex-col" style={{ gap: "2px" }}>
                  <div style={{ fontWeight: 500, fontSize: "14px", fontFamily: "monospace" }}>{tool.name}</div>
                  <div style={{ fontSize: "12px", color: "var(--md-sys-color-outline)" }}>{tool.purpose}</div>
                  {!tool.installed && (
                    <div style={{ fontSize: "12px", color: "var(--md-sys-color-error)" }}>{tool.hint}</div>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
