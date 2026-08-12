import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // The dev server proxies the file API to the node server so `npm run dev` behaves like prod.
  server: {
    port: 7874,
    proxy: {
      "/scene": "http://127.0.0.1:7873",
      "/scenes": "http://127.0.0.1:7873",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 4096,
  },
});
