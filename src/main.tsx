import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { startLogListener } from "./lib/logStore";

// Register the backend log listener once, before any view mounts, so
// commands that run while the Logs screen is closed are still captured.
startLogListener();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
