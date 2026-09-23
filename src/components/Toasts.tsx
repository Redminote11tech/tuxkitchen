import { X, CheckCircle2, AlertTriangle, Info } from "lucide-react";
import { useToasts, dismissToast } from "../lib/toastStore";
import { useBusy } from "../lib/busyStore";

const ICONS = {
  info: <Info size={16} color="var(--md-sys-color-primary)" />,
  error: <AlertTriangle size={16} color="var(--md-sys-color-error)" />,
  success: <CheckCircle2 size={16} color="var(--md-sys-color-primary)" />,
};

export function Toasts() {
  const toasts = useToasts();
  const busy = useBusy();

  return (
    <>
      {busy.active && (
        <div className="busy-bar-wrap" title={busy.label}>
          <div className="busy-bar" />
          <span className="busy-label">{busy.label}</span>
        </div>
      )}
      <div className="toast-stack">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`}>
            {ICONS[t.kind]}
            <span className="toast-msg">{t.message}</span>
            <button className="icon-btn" style={{ padding: 4 }} onClick={() => dismissToast(t.id)}>
              <X size={14} />
            </button>
          </div>
        ))}
      </div>
    </>
  );
}
