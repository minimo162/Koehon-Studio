use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_ADDR: &str = "127.0.0.1:18083";
const SAMPLE_RATE: u32 = 48_000;

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
    engine: &'static str,
    voices: Vec<Voice>,
}

#[derive(Debug, Serialize)]
struct Voice {
    id: &'static str,
    name: &'static str,
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
}

fn main() -> std::io::Result<()> {
    let addr = env::var("KOEHON_TTS_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let listener = TcpListener::bind(&addr)?;
    println!("koehon tts sidecar listening on http://{addr}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| {
                    if let Err(error) = handle_connection(stream) {
                        eprintln!("request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }

    Ok(())
}

fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
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
            write_json(&mut stream, 200, &health_response())
        }
        line if line.starts_with("OPTIONS ") => write_empty(&mut stream, 204),
        line if line.starts_with("POST /synthesize ") => synthesize(&mut stream, body),
        _ => write_json(
            &mut stream,
            404,
            &ErrorResponse {
                ok: false,
                error: "not found".to_string(),
            },
        ),
    }
}

fn synthesize(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
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
                },
            );
        }
    };

    if request.text.trim().is_empty() {
        return write_json(
            stream,
            400,
            &ErrorResponse {
                ok: false,
                error: "text is empty".to_string(),
            },
        );
    }

    let output_path = normalize_output_path(&request.output_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let duration_ms = (request.text.chars().count() as u32 * 45).clamp(350, 5_000);
    let frequency = match request.voice.as_deref() {
        Some("default") | None => 440.0,
        Some(_) => 523.25,
    };
    let seed_offset = request.seed.unwrap_or(0) as f32 % 37.0;
    write_test_tone_wav(&output_path, duration_ms, frequency + seed_offset)?;

    let response = SynthesizeResponse {
        ok: true,
        request_id: request.request_id,
        audio_path: output_path.to_string_lossy().to_string(),
        sample_rate: SAMPLE_RATE,
        elapsed_seconds: started.elapsed().as_secs_f32(),
    };
    write_json(stream, 200, &response)
}

fn health_response() -> HealthResponse {
    HealthResponse {
        ok: true,
        engine: "koehon-test-tone",
        voices: vec![Voice {
            id: "default",
            name: "Default test tone",
        }],
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

fn write_test_tone_wav(path: &Path, duration_ms: u32, frequency: f32) -> std::io::Result<()> {
    let samples = (SAMPLE_RATE as u64 * duration_ms as u64 / 1000) as u32;
    let data_bytes = samples * 2;
    let mut file = fs::File::create(path)?;

    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;

    for index in 0..samples {
        let t = index as f32 / SAMPLE_RATE as f32;
        let envelope = if index < 480 {
            index as f32 / 480.0
        } else if samples.saturating_sub(index) < 480 {
            samples.saturating_sub(index) as f32 / 480.0
        } else {
            1.0
        };
        let sample = (t * frequency * std::f32::consts::TAU).sin() * 0.18 * envelope;
        let pcm = (sample * i16::MAX as f32).round() as i16;
        file.write_all(&pcm.to_le_bytes())?;
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
