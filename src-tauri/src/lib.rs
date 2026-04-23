use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf, process::Command};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WavMergeInput {
    File {
        path: String,
    },
    // `rename_all` on the enum itself only renames variant names; fields
    // inside struct-like variants keep their Rust snake_case unless we
    // re-declare rename_all on the variant. Frontend sends `durationMs`.
    #[serde(rename_all = "camelCase")]
    Silence {
        duration_ms: u32,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WavMergeOutput {
    output_path: String,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    duration_ms: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PortCleanupResult {
    killed_pids: Vec<u32>,
    errors: Vec<String>,
}

#[derive(Debug)]
struct WavData {
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    data: Vec<u8>,
}

#[tauri::command]
fn merge_wav_files(
    inputs: Vec<WavMergeInput>,
    output_path: String,
) -> Result<WavMergeOutput, String> {
    if inputs.is_empty() {
        return Err("結合する音声ファイルがありません。".to_string());
    }

    let mut output = Vec::new();
    let mut format: Option<(u32, u16, u16)> = None;

    for input in inputs {
        match input {
            WavMergeInput::File { path } => {
                let wav = read_wav(PathBuf::from(&path))?;
                let expected = match format {
                    Some(expected) => expected,
                    None => {
                        let current = (wav.sample_rate, wav.channels, wav.bits_per_sample);
                        format = Some(current);
                        current
                    }
                };
                if expected != (wav.sample_rate, wav.channels, wav.bits_per_sample) {
                    return Err(format!(
                        "WAV形式が一致しません: {path} は {}Hz/{}ch/{}bit です。",
                        wav.sample_rate, wav.channels, wav.bits_per_sample
                    ));
                }
                output.extend_from_slice(&wav.data);
            }
            WavMergeInput::Silence { duration_ms } => {
                let (sample_rate, channels, bits_per_sample) = format.unwrap_or((48_000, 1, 16));
                format = Some((sample_rate, channels, bits_per_sample));
                let bytes_per_sample = u32::from(bits_per_sample) / 8;
                let frame_bytes = u32::from(channels) * bytes_per_sample;
                let frames = u64::from(sample_rate) * u64::from(duration_ms) / 1000;
                let byte_len = frames
                    .checked_mul(u64::from(frame_bytes))
                    .ok_or_else(|| "無音データが大きすぎます。".to_string())?;
                output.resize(output.len() + byte_len as usize, 0);
            }
        }
    }

    let (sample_rate, channels, bits_per_sample) =
        format.ok_or_else(|| "WAV形式を決定できませんでした。".to_string())?;
    let output_path = PathBuf::from(output_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_wav(
        &output_path,
        sample_rate,
        channels,
        bits_per_sample,
        &output,
    )?;

    let bytes_per_second = sample_rate
        .saturating_mul(u32::from(channels))
        .saturating_mul(u32::from(bits_per_sample) / 8);
    let duration_ms = if bytes_per_second == 0 {
        0
    } else {
        ((output.len() as u64 * 1000) / u64::from(bytes_per_second)) as u32
    };

    Ok(WavMergeOutput {
        output_path: output_path.to_string_lossy().to_string(),
        sample_rate,
        channels,
        bits_per_sample,
        duration_ms,
    })
}

#[tauri::command]
fn clear_stale_sidecar_port(port: u16) -> Result<PortCleanupResult, String> {
    if port != 18_083 {
        return Err("sidecar以外のポートは停止できません。".to_string());
    }

    clear_listening_processes_on_port(port)
}

#[cfg(windows)]
fn clear_listening_processes_on_port(port: u16) -> Result<PortCleanupResult, String> {
    let output = Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
        .map_err(|error| format!("netstatを実行できませんでした: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "netstatが失敗しました: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pids = parse_netstat_listening_pids(&stdout, port);
    let mut killed_pids = Vec::new();
    let mut errors = Vec::new();

    for pid in pids {
        let result = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
        match result {
            Ok(output) if output.status.success() => killed_pids.push(pid),
            Ok(output) => errors.push(format!(
                "PID {pid} を停止できませんでした: {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => errors.push(format!("PID {pid} の停止に失敗しました: {error}")),
        }
    }

    Ok(PortCleanupResult {
        killed_pids,
        errors,
    })
}

#[cfg(not(windows))]
fn clear_listening_processes_on_port(port: u16) -> Result<PortCleanupResult, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("lsof -tiTCP:{port} -sTCP:LISTEN 2>/dev/null"))
        .output()
        .map_err(|error| format!("lsofを実行できませんでした: {error}"))?;
    let mut killed_pids = Vec::new();
    let mut errors = Vec::new();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(pid) = line.trim().parse::<u32>() else {
            continue;
        };
        let result = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
        match result {
            Ok(output) if output.status.success() => killed_pids.push(pid),
            Ok(output) => errors.push(format!(
                "PID {pid} を停止できませんでした: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => errors.push(format!("PID {pid} の停止に失敗しました: {error}")),
        }
    }

    Ok(PortCleanupResult {
        killed_pids,
        errors,
    })
}

#[cfg(windows)]
fn parse_netstat_listening_pids(output: &str, port: u16) -> Vec<u32> {
    let mut pids = std::collections::BTreeSet::new();
    let port_suffix = format!(":{port}");

    for line in output.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 5 {
            continue;
        }
        if !columns[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !columns[1].ends_with(&port_suffix) {
            continue;
        }
        if !columns[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        if let Ok(pid) = columns[4].parse::<u32>() {
            pids.insert(pid);
        }
    }

    pids.into_iter().collect()
}

fn read_wav(path: PathBuf) -> Result<WavData, String> {
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("WAVファイルではありません: {}", path.display()));
    }

    let mut cursor = 12usize;
    let mut sample_rate = None;
    let mut channels = None;
    let mut bits_per_sample = None;
    let mut data = None;

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let len = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        cursor += 8;
        if cursor + len > bytes.len() {
            return Err(format!("WAVチャンクが壊れています: {}", path.display()));
        }

        match id {
            b"fmt " => {
                if len < 16 {
                    return Err(format!("fmtチャンクが短すぎます: {}", path.display()));
                }
                let audio_format = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
                if audio_format != 1 {
                    return Err(format!("PCM WAVのみ対応しています: {}", path.display()));
                }
                channels = Some(u16::from_le_bytes([bytes[cursor + 2], bytes[cursor + 3]]));
                sample_rate = Some(u32::from_le_bytes([
                    bytes[cursor + 4],
                    bytes[cursor + 5],
                    bytes[cursor + 6],
                    bytes[cursor + 7],
                ]));
                bits_per_sample =
                    Some(u16::from_le_bytes([bytes[cursor + 14], bytes[cursor + 15]]));
            }
            b"data" => data = Some(bytes[cursor..cursor + len].to_vec()),
            _ => {}
        }

        cursor += len + (len % 2);
    }

    let sample_rate =
        sample_rate.ok_or_else(|| format!("sample rateがありません: {}", path.display()))?;
    let channels = channels.ok_or_else(|| format!("channelsがありません: {}", path.display()))?;
    let bits_per_sample = bits_per_sample
        .ok_or_else(|| format!("bits per sampleがありません: {}", path.display()))?;
    if bits_per_sample != 16 {
        return Err(format!(
            "16bit PCM WAVのみ対応しています: {}",
            path.display()
        ));
    }

    Ok(WavData {
        sample_rate,
        channels,
        bits_per_sample,
        data: data.ok_or_else(|| format!("dataチャンクがありません: {}", path.display()))?,
    })
}

fn write_wav(
    path: &PathBuf,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    data: &[u8],
) -> Result<(), String> {
    let data_len = u32::try_from(data.len()).map_err(|_| "WAVが大きすぎます。".to_string())?;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let mut file = fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(b"RIFF").map_err(|error| error.to_string())?;
    file.write_all(&(36 + data_len).to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(b"WAVEfmt ")
        .map_err(|error| error.to_string())?;
    file.write_all(&16u32.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&channels.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&sample_rate.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&byte_rate.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&block_align.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&bits_per_sample.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(b"data").map_err(|error| error.to_string())?;
    file.write_all(&data_len.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(data).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            merge_wav_files,
            clear_stale_sidecar_port
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn merges_pcm_wav_files_and_silence() {
        let dir = temp_dir("merge_pcm_wav_files_and_silence");
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        let output = dir.join("merged.wav");

        write_wav(&first, 48_000, 1, 16, &[1, 0, 2, 0]).unwrap();
        write_wav(&second, 48_000, 1, 16, &[3, 0]).unwrap();

        let result = merge_wav_files(
            vec![
                WavMergeInput::File {
                    path: first.to_string_lossy().to_string(),
                },
                WavMergeInput::Silence { duration_ms: 1 },
                WavMergeInput::File {
                    path: second.to_string_lossy().to_string(),
                },
            ],
            output.to_string_lossy().to_string(),
        )
        .unwrap();

        let merged = read_wav(PathBuf::from(result.output_path)).unwrap();
        assert_eq!(merged.sample_rate, 48_000);
        assert_eq!(merged.channels, 1);
        assert_eq!(merged.bits_per_sample, 16);
        assert_eq!(merged.data.len(), 4 + 96 + 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_mismatched_wav_formats() {
        let dir = temp_dir("rejects_mismatched_wav_formats");
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        let output = dir.join("merged.wav");

        write_wav(&first, 48_000, 1, 16, &[1, 0]).unwrap();
        write_wav(&second, 44_100, 1, 16, &[2, 0]).unwrap();

        let error = merge_wav_files(
            vec![
                WavMergeInput::File {
                    path: first.to_string_lossy().to_string(),
                },
                WavMergeInput::File {
                    path: second.to_string_lossy().to_string(),
                },
            ],
            output.to_string_lossy().to_string(),
        )
        .unwrap_err();

        assert!(error.contains("WAV形式が一致しません"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deserializes_camelcase_silence_input() {
        // Regression: frontend sends `durationMs`, enum-level rename_all
        // doesn't reach variant fields — we re-declared it on the variant.
        let json = r#"[{"type":"file","path":"a.wav"},{"type":"silence","durationMs":250}]"#;
        let parsed: Vec<WavMergeInput> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 2);
        match &parsed[1] {
            WavMergeInput::Silence { duration_ms } => assert_eq!(*duration_ms, 250),
            other => panic!("expected Silence, got {other:?}"),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("koehon-studio-{name}-{nonce}"))
    }
}
