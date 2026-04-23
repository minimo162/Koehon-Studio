// Koehon Studio — TTS sidecar launcher.
//
// Tauri lists a single binary via `externalBin`. That binary used to
// host the full ORT/MOSS inference pipeline; the project now runs
// Irodori-TTS from Python, so this launcher's only job is to:
//
//   1. Find the bundled Python runtime (python-build-standalone +
//      pip-installed torch / irodori / dacvae).
//   2. Spawn `python.exe server.py <original args>`.
//   3. Forward stdio and the exit code.
//
// The CLI surface the Tauri frontend sends (--model-dir, --codec-dir,
// --cpu-threads …) is passed through unchanged so the frontend didn't
// need to learn about the Python swap.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Where to look for the bundled python interpreter, relative to the
/// launcher executable's directory. First hit wins.
///
/// The first three paths match Tauri 2's resource layout on Windows
/// (resources are copied next to the main exe). The last entry is a
/// dev fallback so `cargo run` works from the source tree.
#[cfg(windows)]
const PYTHON_CANDIDATES: &[&str] = &[
    "python-runtime/python/python.exe",
    "resources/python-runtime/python/python.exe",
    "resources/_up_/python-runtime/python/python.exe",
    "../../python-runtime/python/python.exe",
    "../../../src-tauri/python-runtime/python/python.exe",
];

#[cfg(not(windows))]
const PYTHON_CANDIDATES: &[&str] = &[
    "python-runtime/python/bin/python3",
    "resources/python-runtime/python/bin/python3",
    "resources/_up_/python-runtime/python/bin/python3",
    "../../python-runtime/python/bin/python3",
    "../../../src-tauri/python-runtime/python/bin/python3",
];

const SERVER_CANDIDATES: &[&str] = &[
    "python-runtime/server.py",
    "resources/python-runtime/server.py",
    "resources/_up_/python-runtime/server.py",
    "../../python-runtime/server.py",
    "../../../src-tauri/python-runtime/server.py",
];

fn resolve_bundle(exe_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    for (py_rel, srv_rel) in PYTHON_CANDIDATES.iter().zip(SERVER_CANDIDATES.iter()) {
        let py = exe_dir.join(py_rel);
        let srv = exe_dir.join(srv_rel);
        if py.is_file() && srv.is_file() {
            return Some((py, srv));
        }
    }
    None
}

/// Dev fallback: this Rust crate lives at <repo>/native-tts, the
/// companion Python sidecar at <repo>/native-tts-python. When running
/// via `cargo run` from the crate dir, walk up until we find it.
fn resolve_dev_fallback(exe_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut cursor = exe_dir;
    for _ in 0..6 {
        let candidate = cursor.join("native-tts-python").join("server.py");
        if candidate.is_file() {
            let py = PathBuf::from(if cfg!(windows) { "python.exe" } else { "python3" });
            return Some((py, candidate));
        }
        cursor = cursor.parent()?;
    }
    None
}

fn main() -> ExitCode {
    let exe = match env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("koehon-tts-sidecar: current_exe() failed: {err}");
            return ExitCode::from(2);
        }
    };
    let exe_dir = exe.parent().unwrap_or(Path::new("."));

    let (python, server) = match resolve_bundle(exe_dir).or_else(|| resolve_dev_fallback(exe_dir)) {
        Some(pair) => pair,
        None => {
            eprintln!(
                "koehon-tts-sidecar: bundled Python runtime not found next to '{}'. \
                 Expected python-runtime/ sibling directory with python/ and server.py.",
                exe_dir.display()
            );
            return ExitCode::from(3);
        }
    };

    let forwarded: Vec<OsString> = env::args_os().skip(1).collect();

    // The bundled python-build-standalone is self-contained (PYTHONHOME
    // relative to the exe), so we don't set PYTHONHOME here. We DO
    // forward encoding hints so Japanese stdout prints correctly on
    // Windows, which is the only supported target for now.
    let status = Command::new(&python)
        .arg(&server)
        .args(&forwarded)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .status();

    match status {
        Ok(s) => match s.code() {
            Some(code) if (0..=255).contains(&code) => ExitCode::from(code as u8),
            Some(_) | None => ExitCode::FAILURE,
        },
        Err(err) => {
            eprintln!(
                "koehon-tts-sidecar: failed to spawn {} {}: {err}",
                python.display(),
                server.display()
            );
            ExitCode::from(4)
        }
    }
}
