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
    // Two webviews to satisfy: WKWebView on macOS and WebView2 (Chromium) on
    // Windows. Targeting only safari18 would let through syntax Safari has and
    // an older pinned WebView2 does not, and the failure mode is a blank window
    // with a syntax error in a console nobody opens.
    target: process.platform === "win32" ? "chrome110" : "safari18",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "index.html"),
        overlay: resolve(import.meta.dirname, "overlay.html"),
        pill: resolve(import.meta.dirname, "pill.html"),
      },
    },
  },
});
