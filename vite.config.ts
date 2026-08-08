import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";

// Tauri expects a fixed port and must not fall back to another one.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  resolve: {
    alias: { "@": resolve(import.meta.dirname, "src") },
  },
  build: {
    target: "safari18",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "index.html"),
        overlay: resolve(import.meta.dirname, "overlay.html"),
        pill: resolve(import.meta.dirname, "pill.html"),
      },
    },
  },
});
