import { useEffect, useRef } from "react";
import { getLogs, getSeq, subscribeLogs, } from "../lib/logStore";
import { useSyncExternalStore } from "react";

function logClass(line: string): string {
  if (line.includes("[Error]") || line.includes("[err]")) return "log-err";
  if (line.startsWith("[System]")) return "log-sys";
  return "";
}

export function Console() {
  // useSyncExternalStore keeps the console in sync with the global buffer:
  // logs emitted while another view is open are already in the store, so
  // switching views no longer resets or drops history.
  useSyncExternalStore(
    (fn) => subscribeLogs(fn),
    () => getSeq(),
  );
  const logs = getLogs();
  const consoleEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    consoleEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs.length]);

  return (
    <div className="console-container">
      {logs.map((log, index) => (
        <div key={index} className={logClass(log)}>{log}</div>
      ))}
      <div ref={consoleEndRef} />
    </div>
  );
}
