import { getCurrentWindow } from "@tauri-apps/api/window";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { inTauri } from "./api";
import { LiveWindow } from "./LiveWindow";
import "./styles.css";

// The pop-out live window loads the same page; its label picks the view.
const isLiveWindow = inTauri && getCurrentWindow().label === "live";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(<StrictMode>{isLiveWindow ? <LiveWindow /> : <App />}</StrictMode>);
}
