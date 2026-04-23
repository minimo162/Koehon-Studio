#!/usr/bin/env node
/**
 * Build a self-contained Python runtime for the Koehon Studio sidecar.
 *
 * Downloads python-build-standalone, pip-installs CPU PyTorch + Irodori-TTS
 * + Semantic-DACVAE into the embedded interpreter, and drops server.py
 * alongside. Tauri then picks up the resulting `python-runtime/`
 * directory as a resource (see src-tauri/tauri.windows.conf.json) and
 * the Rust launcher sidecar spawns `python.exe server.py` at runtime.
 *
 * Windows-only for now.
 *
 * Usage:
 *   node scripts/bundle-python.mjs --out src-tauri/python-runtime
 *
 * Options:
 *   --out <dir>          Output directory (default: src-tauri/python-runtime)
 *   --python-tag <tag>   python-build-standalone release tag (default: pinned)
 *   --python-version <v> cpython version string (default: pinned)
 *   --torch-version <v>  torch wheel version (default: pinned)
 *   --keep-cache         Keep downloaded tarballs in .cache/
 */
import { execFileSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { arch, platform, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const idx = args.indexOf(name);
  return idx >= 0 && idx + 1 < args.length ? args[idx + 1] : fallback;
};
const flag = (name) => args.includes(name);

// --- pinned versions -----------------------------------------------------
// When updating: check https://github.com/astral-sh/python-build-standalone/releases
// and pick a recent install_only_stripped asset. The version-tag pair must
// exist as a published archive: cpython-${PYTHON_VERSION}+${PYTHON_TAG}-...
const PYTHON_TAG = opt("--python-tag", "20260414");
const PYTHON_VERSION = opt("--python-version", "3.11.15");
const TORCH_VERSION = opt("--torch-version", "2.5.1");
const TORCHAUDIO_VERSION = opt("--torchaudio-version", "2.5.1");

const OUT_DIR = resolve(repoRoot, opt("--out", "src-tauri/python-runtime"));
const CACHE_DIR = resolve(repoRoot, ".cache/python-bundle");

// --- helpers -------------------------------------------------------------

function log(msg) {
  process.stdout.write(`[bundle-python] ${msg}\n`);
}

function run(cmd, argv, options = {}) {
  log(`$ ${cmd} ${argv.join(" ")}`);
  execFileSync(cmd, argv, { stdio: "inherit", ...options });
}

function bytesOf(p) {
  try {
    return statSync(p).size;
  } catch {
    return 0;
  }
}

function humanSize(n) {
  const units = ["B", "KB", "MB", "GB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${v.toFixed(u >= 2 ? 2 : 1)} ${units[u]}`;
}

function download(url, dest) {
  if (existsSync(dest) && bytesOf(dest) > 0) {
    log(`cached ${dest} (${humanSize(bytesOf(dest))})`);
    return;
  }
  mkdirSync(dirname(dest), { recursive: true });
  const tmp = `${dest}.part`;
  // curl is available on every supported CI runner (Windows 2022, macOS, Ubuntu).
  run("curl", ["-fL", "--retry", "3", "--retry-delay", "2", "-o", tmp, url]);
  cpSync(tmp, dest);
  rmSync(tmp, { force: true });
  log(`downloaded ${dest} (${humanSize(bytesOf(dest))})`);
}

function extract(tarPath, outDir) {
  mkdirSync(outDir, { recursive: true });
  // BSD tar on Windows (System32\tar.exe, present since 1803) handles .tar.gz.
  run("tar", ["-xzf", tarPath, "-C", outDir]);
}

function pbsTriple() {
  // python-build-standalone asset triples.
  const a = arch();
  const p = platform();
  if (p === "win32" && a === "x64") return "x86_64-pc-windows-msvc";
  if (p === "linux" && a === "x64") return "x86_64-unknown-linux-gnu";
  if (p === "linux" && a === "arm64") return "aarch64-unknown-linux-gnu";
  if (p === "darwin" && a === "arm64") return "aarch64-apple-darwin";
  if (p === "darwin" && a === "x64") return "x86_64-apple-darwin";
  throw new Error(`unsupported host platform=${p} arch=${a}`);
}

function pythonExe(pythonDir) {
  if (platform() === "win32") return join(pythonDir, "python.exe");
  return join(pythonDir, "bin", "python3");
}

function resolveSitePackages(pythonDir) {
  // python-build-standalone layouts:
  //   Windows:  <pythonDir>/Lib/site-packages/
  //   POSIX:    <pythonDir>/lib/python3.X/site-packages/
  if (platform() === "win32") {
    return join(pythonDir, "Lib", "site-packages");
  }
  const libDir = join(pythonDir, "lib");
  if (!existsSync(libDir)) {
    throw new Error(`site-packages not found under ${pythonDir}`);
  }
  const pyDir = readdirSync(libDir).find((n) => /^python3\.\d+$/.test(n));
  if (!pyDir) {
    throw new Error(`could not locate python3.X directory under ${libDir}`);
  }
  return join(libDir, pyDir, "site-packages");
}

function installFromGitSubdir(gitUrl, subdir, siteDir) {
  // Shallow-clone the repo into the bundle cache, then recursively copy
  // the single package directory into site-packages. Bypasses the
  // upstream's pyproject/setup.py entirely so we don't inherit their
  // packaging quirks (flat-layout discovery errors, pinned
  // torch>=2.10, etc.).
  const cloneDir = join(CACHE_DIR, subdir);
  if (existsSync(cloneDir)) {
    rmSync(cloneDir, { recursive: true, force: true });
  }
  log(`cloning ${gitUrl} → ${cloneDir}`);
  run("git", ["clone", "--depth", "1", "--filter=blob:none", gitUrl, cloneDir]);

  const srcPkgDir = join(cloneDir, subdir);
  if (!existsSync(srcPkgDir)) {
    throw new Error(`expected package directory ${subdir} not found in ${gitUrl}`);
  }
  const dstPkgDir = join(siteDir, subdir);
  if (existsSync(dstPkgDir)) {
    rmSync(dstPkgDir, { recursive: true, force: true });
  }
  log(`copying ${srcPkgDir} → ${dstPkgDir}`);
  cpSync(srcPkgDir, dstPkgDir, { recursive: true });
}

// --- main ---------------------------------------------------------------

function main() {
  if (platform() !== "win32" && !process.env.KOEHON_ALLOW_NON_WINDOWS_BUNDLE) {
    // Windows is the only supported target per the current roadmap; on Linux
    // CI we'd just be smoke-testing. Gate behind an env var so `act`-style
    // local reproduction still works.
    throw new Error(
      "bundle-python currently only targets Windows. Set KOEHON_ALLOW_NON_WINDOWS_BUNDLE=1 to proceed anyway.",
    );
  }

  log(`output → ${OUT_DIR}`);
  if (existsSync(OUT_DIR)) {
    log("clearing existing output directory");
    rmSync(OUT_DIR, { recursive: true, force: true });
  }
  mkdirSync(OUT_DIR, { recursive: true });
  mkdirSync(CACHE_DIR, { recursive: true });

  // 1. Download + extract python-build-standalone
  const triple = pbsTriple();
  const pbsFile =
    `cpython-${PYTHON_VERSION}+${PYTHON_TAG}-${triple}-install_only_stripped.tar.gz`;
  const pbsUrl =
    `https://github.com/astral-sh/python-build-standalone/releases/download/${PYTHON_TAG}/${pbsFile}`;
  const pbsTar = join(CACHE_DIR, pbsFile);
  log(`fetching ${pbsUrl}`);
  download(pbsUrl, pbsTar);

  extract(pbsTar, OUT_DIR);
  // The archive unpacks to a top-level `python/` directory already.
  const pythonDir = join(OUT_DIR, "python");
  const py = pythonExe(pythonDir);
  if (!existsSync(py)) {
    throw new Error(`python-build-standalone layout unexpected: ${py} missing`);
  }
  log(`python at ${py}`);

  // 2. Upgrade pip, install CPU torch, other deps, irodori-tts, dacvae
  const envForPip = {
    ...process.env,
    PIP_DISABLE_PIP_VERSION_CHECK: "1",
    PIP_NO_WARN_SCRIPT_LOCATION: "1",
    PYTHONDONTWRITEBYTECODE: "1",
  };

  run(py, ["-m", "pip", "install", "--upgrade", "pip"], { env: envForPip });

  // torch/torchaudio from the PyTorch CPU wheel index. We pin exactly to
  // keep the bundle deterministic; update TORCH_VERSION above when bumping.
  run(
    py,
    [
      "-m",
      "pip",
      "install",
      "--index-url",
      "https://download.pytorch.org/whl/cpu",
      `torch==${TORCH_VERSION}`,
      `torchaudio==${TORCHAUDIO_VERSION}`,
    ],
    { env: envForPip },
  );

  // HTTP + misc deps from regular PyPI.
  run(
    py,
    [
      "-m",
      "pip",
      "install",
      "-r",
      join(repoRoot, "native-tts-python", "requirements.txt"),
    ],
    { env: envForPip },
  );

  // Irodori's repo has `irodori_tts/` and `configs/` at the top level,
  // which makes setuptools' flat-layout auto-discovery refuse the
  // install with "Multiple top-level packages discovered". We don't
  // actually need a wheel — the server imports `irodori_tts` only —
  // so skip pip packaging and copy the package directory straight into
  // the bundle's site-packages. Same for DACVAE (its `dacvae/`
  // subdirectory is the importable package).
  const siteDir = resolveSitePackages(pythonDir);
  log(`site-packages → ${siteDir}`);
  installFromGitSubdir(
    "https://github.com/Aratako/Irodori-TTS.git",
    "irodori_tts",
    siteDir,
  );
  installFromGitSubdir(
    "https://github.com/facebookresearch/dacvae.git",
    "dacvae",
    siteDir,
  );

  // 3. Copy server.py into the bundle root alongside python/
  cpSync(
    join(repoRoot, "native-tts-python", "server.py"),
    join(OUT_DIR, "server.py"),
  );

  // 4. Write a tiny manifest describing what's inside — useful when
  //    diagnosing field issues without shelling into python.exe.
  const manifest = {
    generated_at: new Date().toISOString(),
    python_version: PYTHON_VERSION,
    python_tag: PYTHON_TAG,
    torch_version: TORCH_VERSION,
    torchaudio_version: TORCHAUDIO_VERSION,
    triple,
  };
  writeFileSync(
    join(OUT_DIR, "bundle-manifest.json"),
    JSON.stringify(manifest, null, 2),
  );

  // 5. Clean: drop __pycache__ trees (regeneratable, adds ~100 MB) and
  //    .dist-info RECORD hash files (harmless but bloaty). Keeps the
  //    installer smaller without breaking imports.
  pruneBundle(OUT_DIR);

  // 6. Summary
  const finalSize = dirSize(OUT_DIR);
  log(`bundle ready at ${OUT_DIR} · ${humanSize(finalSize)}`);
  if (!flag("--keep-cache")) {
    rmSync(CACHE_DIR, { recursive: true, force: true });
  }
}

function pruneBundle(root) {
  const removeRecursive = (dir, matcher) => {
    let removed = 0;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (matcher(entry.name)) {
          rmSync(full, { recursive: true, force: true });
          removed += 1;
          continue;
        }
        removed += removeRecursive(full, matcher);
      }
    }
    return removed;
  };
  const cacheRemoved = removeRecursive(root, (name) => name === "__pycache__");
  log(`pruned ${cacheRemoved} __pycache__ directories`);
}

function dirSize(dir) {
  let total = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) total += dirSize(full);
    else total += bytesOf(full);
  }
  return total;
}

try {
  main();
} catch (err) {
  log(`FAILED: ${err?.message ?? err}`);
  process.exit(1);
}
