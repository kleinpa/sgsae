import { defineConfig } from "vite";

// Under Bazel every input is a symlink into the execroot. Vite resolves
// symlinks to their real paths by default, which puts the entry HTML outside
// `root` and the output directory outside the writable sandbox. Keeping the
// symlinks makes every path stay sandbox-relative, which is what rules_js
// expects.
export default defineConfig({
  resolve: {
    preserveSymlinks: true,
  },
  // Programs live here and are copied to the site root unchanged, so an
  // address resolves to the same path after a build as it does in dev.
  publicDir: "public",
  // An address like /system/seq is a page, not a file: serve the app for any
  // path with no extension and let it read the path itself.
  appType: "spa",
  // Bind all interfaces, not just loopback, so the devcontainer's forwarded
  // port can reach the server from the host browser.
  server: {
    host: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
