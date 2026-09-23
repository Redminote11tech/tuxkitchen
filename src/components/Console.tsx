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
  const stickToBottom = useRef(true);

  // Follow new output only while the user is already at the bottom, and do
  // it instantly: a smooth-scroll animation per streamed line is jank.
  useEffect(() => {
    const end = consoleEndRef.current;
    const container = end?.parentElement;
    if (!end || !container) return;
    if (stickToBottom.current) {
      end.scrollIntoView({ block: "end" });
    }
  }, [logs.length]);

  const onScroll = () => {
    const end = consoleEndRef.current;
    const container = end?.parentElement;
    if (!container) return;
    const distance = container.scrollHeight - container.scrollTop - container.clientHeight;
    stickToBottom.current = distance < 80;
  };

  return (
    <div className="console-container" onScroll={onScroll}>
      {logs.map((log, index) => (
        <div key={index} className={logClass(log)}>{log}</div>
      ))}
      <div ref={consoleEndRef} />
    </div>
  );
}
