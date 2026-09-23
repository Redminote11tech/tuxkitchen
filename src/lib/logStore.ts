import { listen } from "@tauri-apps/api/event";

// Global log buffer so the console keeps its history across view switches:
// logs emitted while the Logs screen is closed stay in the ring buffer
// instead of being dropped with the unmounted listener.
const MAX_LINES = 5000;

let lines: string[] = [
  "[System] TuxKitchen initialized.",
  "[System] Waiting for commands...",
];
const subscribers = new Set<() => void>();

let seq = 0;
export function pushLog(line: string) {
  lines = [...lines.slice(-(MAX_LINES - 1)), line];
  seq++;
  subscribers.forEach((fn) => fn());
}

export function getLogs(): string[] {
  return lines;
}

export function subscribeLogs(fn: () => void): () => void {
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

export function getSeq(): number {
  return seq;
}

let started = false;
export function startLogListener() {
  if (started) return;
  started = true;
  try {
    listen<string>("log-event", (event) => pushLog(event.payload)).catch((e) =>
      console.error("log listener error:", e),
    );
  } catch (e) {
    // Running in a plain browser (vite dev without Tauri): ignore.
    console.error("log listener setup error (mocking if not in Tauri):", e);
  }
}
