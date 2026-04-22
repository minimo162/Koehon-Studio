mod engine;

use engine::{
    moss_onnx, moss_tts_nano, test_tone::TestToneEngine, EngineDiagnostic, SynthError, SynthInput,
    TtsEngine, VoiceInfo,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const DEFAULT_ADDR: &str = "127.0.0.1:18083";

#[derive(Debug, Deserialize)]
struct SynthesizeRequest {
    request_id: String,
    text: String,
    voice: Option<String>,
    seed: Option<u64>,
    output_path: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    engine: String,
    engine_name: String,
    sample_rate: u32,
    voices: Vec<VoiceInfo>,
    diagnostics: Vec<EngineDiagnostic>,
}

#[derive(Debug, Serialize)]
struct SynthesizeResponse {
    ok: bool,
    request_id: String,
    audio_path: String,
    sample_rate: u32,
    elapsed_seconds: f32,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
    code: Option<String>,
}

struct SidecarState {
    engine: Arc<dyn TtsEngine>,
    startup_diagnostics: Vec<EngineDiagnostic>,
}

struct CliArgs {
    host: String,
    model_dir: Option<PathBuf>,
    codec_dir: Option<PathBuf>,
    ort_dylib: Option<PathBuf>,
    cpu_threads: u16,
}

fn main() -> std::io::Result<()> {
    let args = parse_args();
    let state = initialize_engine(&args);
    let listener = TcpListener::bind(&args.host)?;
    println!(
        "koehon tts sidecar listening on http://{} engine={}",
        args.host,
        state.engine.id()
    );
    let state = Arc::new(state);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, state) {
                        eprintln!("request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }
    Ok(())
}

fn parse_args() -> CliArgs {
    let mut host = env::var("KOEHON_TTS_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let mut model_dir: Option<PathBuf> = env::var_os("KOEHON_MODEL_DIR").map(PathBuf::from);
    let mut codec_dir: Option<PathBuf> = env::var_os("KOEHON_CODEC_DIR").map(PathBuf::from);
    let mut ort_dylib: Option<PathBuf> = env::var_os("ORT_DYLIB_PATH").map(PathBuf::from);
    let mut cpu_threads: u16 = env::var("KOEHON_CPU_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => {
                if let Some(value) = iter.next() {
                    host = value;
                }
            }
            "--model-dir" => {
                if let Some(value) = iter.next() {
                    model_dir = Some(PathBuf::from(value));
                }
            }
            "--codec-dir" => {
                if let Some(value) = iter.next() {
                    codec_dir = Some(PathBuf::from(value));
                }
            }
            "--ort-dylib" => {
                if let Some(value) = iter.next() {
                    ort_dylib = Some(PathBuf::from(value));
                }
            }
            "--cpu-threads" => {
                if let Some(value) = iter.next() {
                    if let Ok(parsed) = value.parse() {
                        cpu_threads = parsed;
                    }
                }
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => eprintln!("unknown argument ignored: {other}"),
        }
    }
    CliArgs {
        host,
        model_dir,
        codec_dir,
        ort_dylib,
        cpu_threads,
    }
}

fn print_usage() {
    println!("koehon-tts-sidecar");
    println!();
    println!("USAGE:");
    println!("  koehon-tts-sidecar [--host HOST:PORT] [--model-dir DIR]");
    println!("                     [--ort-dylib PATH] [--cpu-threads N]");
    println!();
    println!("ENV:");
    println!("  KOEHON_TTS_ADDR         Equivalent to --host");
    println!("  KOEHON_MODEL_DIR        Equivalent to --model-dir");
    println!("  ORT_DYLIB_PATH          Equivalent to --ort-dylib");
    println!("  KOEHON_CPU_THREADS      Equivalent to --cpu-threads");
}

fn initialize_engine(args: &CliArgs) -> SidecarState {
    let mut diagnostics: Vec<EngineDiagnostic> = Vec::new();

    // 1. MOSS-TTS-Nano multi-stage layout (tts_browser_onnx_meta.json present)
    if let Some(model_dir) = args.model_dir.as_deref() {
        if moss_tts_nano::is_moss_layout(model_dir) {
            let outcome = moss_tts_nano::try_load(model_dir, args.codec_dir.as_deref(), args.cpu_threads);
            diagnostics.extend(outcome.diagnostics);
            if let Some(engine) = outcome.engine {
                return SidecarState {
                    engine: Arc::new(engine),
                    startup_diagnostics: diagnostics,
                };
            }
            let reason = diagnostics
                .last()
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "MOSS-TTS-Nano レイアウトの読み込みに失敗しました".to_string());
            return SidecarState {
                engine: Arc::new(TestToneEngine::with_reason(reason)),
                startup_diagnostics: diagnostics,
            };
        }
    }

    // 2. Generic single-file layout (model.onnx + tokenizer.json + config.json)
    let outcome = moss_onnx::try_load(
        args.model_dir.as_deref(),
        args.ort_dylib.as_deref(),
        args.cpu_threads,
    );
    if let Some(diag) = outcome.diagnostic {
        diagnostics.push(diag);
    }
    let engine: Arc<dyn TtsEngine> = match outcome.engine {
        Some(engine) => Arc::new(engine),
        None => {
            let reason = diagnostics
                .last()
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "TTSモデルを読み込めませんでした".to_string());
            Arc::new(TestToneEngine::with_reason(reason))
        }
    };
    SidecarState {
        engine,
        startup_diagnostics: diagnostics,
    }
}

fn handle_connection(mut stream: TcpStream, state: Arc<SidecarState>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&buffer) {
            let content_length = parse_content_length(&buffer[..header_end]).unwrap_or(0);
            let total = header_end + 4 + content_length;
            while buffer.len() < total {
                let read = stream.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            break;
        }
        if buffer.len() > 1024 * 1024 {
            break;
        }
    }

    let Some(header_end) = find_header_end(&buffer) else {
        return write_json(
            &mut stream,
            400,
            &ErrorResponse {
                ok: false,
                error: "invalid request".to_string(),
                code: Some("http.bad_request".to_string()),
            },
        );
    };
    let request_line = String::from_utf8_lossy(&buffer[..header_end])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let body = &buffer[(header_end + 4)..];

    match request_line.as_str() {
        line if line.starts_with("GET /health ") => {
            write_json(&mut stream, 200, &health_response(&state))
        }
        line if line.starts_with("OPTIONS ") => write_empty(&mut stream, 204),
        line if line.starts_with("POST /synthesize ") => synthesize(&mut stream, body, &state),
        _ => write_json(
            &mut stream,
            404,
            &ErrorResponse {
                ok: false,
                error: "not found".to_string(),
                code: Some("http.not_found".to_string()),
            },
        ),
    }
}

fn synthesize(
    stream: &mut TcpStream,
    body: &[u8],
    state: &SidecarState,
) -> std::io::Result<()> {
    let started = Instant::now();
    let request = match serde_json::from_slice::<SynthesizeRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return write_json(
                stream,
                400,
                &ErrorResponse {
                    ok: false,
                    error: format!("invalid synthesize request: {error}"),
                    code: Some("synth.bad_request".to_string()),
                },
            );
        }
    };

    let synth_input = SynthInput {
        text: request.text.clone(),
        voice: request.voice.clone(),
        seed: request.seed,
    };

    let output_path = normalize_output_path(&request.output_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let result = match state.engine.synthesize(synth_input) {
        Ok(result) => result,
        Err(error) => {
            let (status, code) = error_to_status(&error);
            return write_json(
                stream,
                status,
                &ErrorResponse {
                    ok: false,
                    error: error.to_string(),
                    code: Some(code.to_string()),
                },
            );
        }
    };

    if let Err(error) = write_pcm16_wav(&output_path, &result.samples, result.sample_rate, result.channels) {
        return write_json(
            stream,
            500,
            &ErrorResponse {
                ok: false,
                error: format!("WAVの書き出しに失敗しました: {error}"),
                code: Some("synth.write_failed".to_string()),
            },
        );
    }

    if !output_path.exists() {
        return write_json(
            stream,
            500,
            &ErrorResponse {
                ok: false,
                error: "音声ファイルが作成されましたが見つかりません".to_string(),
                code: Some("synth.missing_output".to_string()),
            },
        );
    }

    let response = SynthesizeResponse {
        ok: true,
        request_id: request.request_id,
        audio_path: output_path.to_string_lossy().to_string(),
        sample_rate: result.sample_rate,
        elapsed_seconds: started.elapsed().as_secs_f32(),
    };
    write_json(stream, 200, &response)
}

fn error_to_status(error: &SynthError) -> (u16, &'static str) {
    match error {
        SynthError::NotReady => (503, "engine.not_ready"),
        SynthError::EmptyText => (400, "synth.empty_text"),
        SynthError::Tokenize(_) => (400, "synth.tokenize_failed"),
        SynthError::BadShape(_) => (500, "synth.bad_shape"),
        SynthError::Inference(_) => (500, "synth.inference_failed"),
    }
}

fn health_response(state: &SidecarState) -> HealthResponse {
    let mut diagnostics = state.startup_diagnostics.clone();
    diagnostics.extend(state.engine.diagnostics());
    HealthResponse {
        ok: true,
        engine: state.engine.id().to_string(),
        engine_name: state.engine.name().to_string(),
        sample_rate: state.engine.sample_rate(),
        voices: state.engine.voices(),
        diagnostics,
    }
}

fn normalize_output_path(path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(candidate)
    }
}

fn write_pcm16_wav(
    path: &Path,
    samples: &[i16],
    sample_rate: u32,
    channels: u16,
) -> std::io::Result<()> {
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_len = (samples.len() * 2) as u32;
    let mut file = fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, value: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)
        .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"json encode failed\"}".to_vec());
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n{}\r\nConnection: close\r\n\r\n",
        body.len(),
        cors_headers()
    )?;
    stream.write_all(&body)
}

fn write_empty(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    let reason = match status {
        204 => "No Content",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\n{}\r\nConnection: close\r\n\r\n",
        cors_headers()
    )
}

fn cors_headers() -> &'static str {
    "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type"
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(headers).lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}
