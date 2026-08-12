import { createRoot } from "react-dom/client";
import App from "./App.jsx";
import "./styles.css";

// Excalidraw fetches its fonts at runtime from `${EXCALIDRAW_ASSET_PATH}fonts/...`. "/" points them
// at dist/fonts (copied in by scripts/copy-fonts.mjs) instead of a CDN, so the tool works offline.
window.EXCALIDRAW_ASSET_PATH = "/";

// No StrictMode on purpose: its dev-only double-mount would run the save/poll loop twice and make
// dev behave unlike the built app this tool actually ships as.
createRoot(document.getElementById("root")).render(<App />);
