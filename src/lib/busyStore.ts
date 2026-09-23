// Global busy state: long backend operations announce themselves so the UI
// can show progress and stop double-firing commands.
import { useSyncExternalStore } from "react";

let busy: { active: boolean; label: string } = { active: false, label: "" };
const subscribers = new Set<() => void>();

function emit() {
  subscribers.forEach((fn) => fn());
}

export async function runBusy<T>(label: string, fn: () => Promise<T>): Promise<T> {
  busy = { active: true, label };
  emit();
  try {
    return await fn();
  } finally {
    busy = { active: false, label: "" };
    emit();
  }
}

export function subscribeBusy(fn: () => void): () => void {
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

export function getBusy(): { active: boolean; label: string } {
  return busy;
}

export function useBusy(): { active: boolean; label: string } {
  return useSyncExternalStore(subscribeBusy, getBusy);
}
