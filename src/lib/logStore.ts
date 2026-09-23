import { listen } from "@tauri-apps/api/event";

// Global log buffer so the console keeps its history across view switches:
// logs emitted while the Logs screen is closed stay in the ring buffer
// instead of being dropped with the unmounted listener.
//
// Incoming lines are batched and flushed on a short timer: a streaming build
// would otherwise re-render the whole list once per line, which on WebKit's
// CPU compositing path turns into visible scroll jank.
const MAX_LINES = 1000;
const FLUSH_MS = 80;

let lines: string[] = [
  "[System] TuxKitchen initialized.",
  "[System] Waiting for commands...",
];
let pending: string[] = [];
let flushTimer: ReturnType<typeof setTimeout> | null = null;
const subscribers = new Set<() => void>();

let seq = 0;
function flush() {
  flushTimer = null;
  if (pending.length === 0) return;
  const batch = pending;
  pending = [];
  lines = [...lines, ...batch].slice(-MAX_LINES);
  seq++;
  subscribers.forEach((fn) => fn());
}

export function pushLog(line: string) {
  pending.push(line);
  if (flushTimer === null) {
    flushTimer = setTimeout(flush, FLUSH_MS);
  }
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

export function clearLogs() {
  lines = ["[System] Log cleared."];
  seq++;
  subscribers.forEach((fn) => fn());
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
