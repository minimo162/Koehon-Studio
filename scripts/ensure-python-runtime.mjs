#!/usr/bin/env node
/**
 * Ensure the embedded Python runtime exists for the TTS sidecar.
 *
 * Windows dev/build runs need `src-tauri/python-runtime`; without it the
 * sidecar launcher cannot start the Python server. Non-Windows hosts skip this
 * because the production Python bundle is Windows-only at the moment.
 */
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { platform } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");
const runtimeDir = resolve(repoRoot, "src-tauri/python-runtime");
const manifestPath = resolve(runtimeDir, "bundle-manifest.json");

if (platform() !== "win32") {
  console.log("[python-runtime] skipping bundle check on non-Windows host");
  process.exit(0);
}

if (existsSync(manifestPath)) {
  console.log("[python-runtime] verifying existing runtime bundle");
} else {
  console.log("[python-runtime] runtime bundle missing; building it now");
}

execFileSync(
  "node",
  [
    resolve(repoRoot, "scripts/bundle-python.mjs"),
    "--out",
    runtimeDir,
    "--reuse-existing",
    "--keep-cache",
  ],
  { stdio: "inherit", cwd: repoRoot },
);
