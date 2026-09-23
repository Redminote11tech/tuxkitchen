// Minimal toast system: replaces blocking window.alert() calls with
// non-blocking, auto-dismissing notifications.
import { useSyncExternalStore } from "react";

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  message: string;
}

let toasts: Toast[] = [];
let nextId = 1;
const subscribers = new Set<() => void>();

function emit() {
  subscribers.forEach((fn) => fn());
}

export function pushToast(kind: Toast["kind"], message: string) {
  const id = nextId++;
  toasts = [...toasts, { id, kind, message }];
  emit();
  setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== id);
    emit();
  }, 4500);
}

export function dismissToast(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export function subscribeToasts(fn: () => void): () => void {
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

export function getToasts(): Toast[] {
  return toasts;
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(subscribeToasts, getToasts);
}

export const toast = {
  info: (m: string) => pushToast("info", m),
  error: (m: string) => pushToast("error", m),
  success: (m: string) => pushToast("success", m),
};
