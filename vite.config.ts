import { defineConfig } from "vite";

// HTML contains only the static document now; Vite owns the visual shell CSS
// and JavaScript entries, so production builds minify and fingerprint them.
// The relative base is required when Tauri loads the bundle from disk.
export default defineConfig({
  root: "src",
  base: "./",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    // 1420 is commonly used by sibling NovaVei desktop checkouts; keep this
    // project on a dedicated port so `tauri dev` can start without fighting them.
    port: 1421,
    strictPort: true,
    // The portable release can be running while development checks use Vite's
    // SSR loader. Its WebView profile contains locked cookies on Windows and
    // is neither source nor a reload input, so never ask the watcher to enter
    // build or release output trees.
    watch: {
      ignored: ["**/src-tauri/target/**", "**/release/**"],
    },
  },
});
