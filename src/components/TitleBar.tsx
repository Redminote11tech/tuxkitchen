import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";

export function TitleBar() {
  const appWindow = getCurrentWindow();

  return (
    <div data-tauri-drag-region className="titlebar">
      <div className="titlebar-title" data-tauri-drag-region>
        TuxKitchen
      </div>
      <div className="titlebar-actions">
        <button
          className="titlebar-btn"
          onClick={() => appWindow.minimize()}
          title="Minimize"
        >
          <Minus size={16} />
        </button>
        <button
          className="titlebar-btn"
          onClick={() => appWindow.toggleMaximize()}
          title="Maximize"
        >
          <Square size={14} />
        </button>
        <button
          className="titlebar-btn close-btn"
          onClick={() => appWindow.close()}
          title="Close"
        >
          <X size={16} />
        </button>
      </div>
    </div>
  );
}
