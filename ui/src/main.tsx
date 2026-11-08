import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Buddy } from "./buddy/Buddy";
import { currentWindowLabel } from "./ipc";
import "./styles.css";

// One bundle, two windows: the main app and the desktop buddy (docs/ROADMAP.md).
const isBuddy = currentWindowLabel() === "buddy";
if (isBuddy) document.documentElement.classList.add("buddy-view");

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(<StrictMode>{isBuddy ? <Buddy /> : <App />}</StrictMode>);
}
