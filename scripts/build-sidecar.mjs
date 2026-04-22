#!/usr/bin/env node
/**
 * Build the native TTS sidecar and copy it into native-tts/sidecars/
 * using the target-triple suffix that Tauri's externalBin expects
 * (e.g. koehon-tts-sidecar-x86_64-pc-windows-msvc.exe).
 *
 * Usage:
 *   node scripts/build-sidecar.mjs            # debug build for the host triple
 *   node scripts/build-sidecar.mjs --release  # release build for the host triple
 *   node scripts/build-sidecar.mjs --target x86_64-pc-windows-msvc --release
 */
import { execSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");
const binaryName = "koehon-tts-sidecar";

const args = process.argv.slice(2);
const isRelease = args.includes("--release");
const targetIndex = args.indexOf("--target");
const explicitTarget = targetIndex >= 0 ? args[targetIndex + 1] : undefined;

function detectHostTriple() {
  const output = execSync("rustc -vV", { encoding: "utf8" });
  const match = output.match(/host:\s*(.+)/);
  if (!match) throw new Error("failed to detect rustc host triple (rustc -vV)");
  return match[1].trim();
}

function main() {
  const triple = explicitTarget ?? detectHostTriple();
  const isWindows = triple.includes("windows");
  const exeExt = isWindows ? ".exe" : "";

  const cargoArgs = ["build", "--manifest-path", "native-tts/Cargo.toml"];
  if (isRelease) cargoArgs.push("--release");
  if (explicitTarget) cargoArgs.push("--target", explicitTarget);

  console.log(`[sidecar] cargo ${cargoArgs.join(" ")}`);
  execSync(`cargo ${cargoArgs.join(" ")}`, { stdio: "inherit", cwd: repoRoot });

  const profile = isRelease ? "release" : "debug";
  const segments = ["native-tts", "target"];
  if (explicitTarget) segments.push(explicitTarget);
  segments.push(profile, `${binaryName}${exeExt}`);
  const srcBinary = resolve(repoRoot, ...segments);

  if (!existsSync(srcBinary)) {
    throw new Error(`sidecar binary not found at ${srcBinary}`);
  }

  const outDir = resolve(repoRoot, "native-tts/sidecars");
  mkdirSync(outDir, { recursive: true });

  const dstBinary = resolve(outDir, `${binaryName}-${triple}${exeExt}`);
  copyFileSync(srcBinary, dstBinary);
  if (!isWindows) {
    try {
      chmodSync(dstBinary, 0o755);
    } catch {
      // ignore on systems without chmod support
    }
  }
  console.log(`[sidecar] copied → ${dstBinary}`);
}

main();
