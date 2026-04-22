use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ndarray::{Array1, Array2, ArrayD, IxDyn};
use ort::session::{builder::GraphOptimizationLevel, Session, SessionInputValue};
use ort::value::Value;
use serde::Deserialize;

use super::{
    tokenizer::Tokenizer, DiagnosticSeverity, EngineDiagnostic, SynthError, SynthInput,
    SynthResult, TtsEngine, VoiceInfo,
};

const DEFAULT_SAMPLE_RATE: u32 = 24_000;

/// Expected file layout inside the `--model-dir`:
///
/// ```text
/// model.onnx              float16/float32 TTS model
/// tokenizer.json          { vocab, unknown_id, bos_id, eos_id, mode }
/// config.json             { sample_rate, channels, voices, dylib_path? }
/// ```
///
/// ONNX I/O shapes expected by this adapter:
///
/// - Input `input_ids` (int64) shape `[1, seq]` — token ids from the tokenizer
/// - Optional input `speaker_id` (int64) shape `[1]` — zero-based voice index
/// - Optional input `seed` (int64) shape `[1]` — sampling seed
/// - Output `audio` (float32) shape `[1, samples]` or `[samples]` — mono PCM in [-1, 1]
pub struct MossOnnxEngine {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    config: MossConfig,
    model_path: PathBuf,
    ort_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossConfig {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u16,
    #[serde(default)]
    pub voices: Vec<MossVoice>,
    #[serde(default)]
    pub speaker_input_name: Option<String>,
    #[serde(default)]
    pub seed_input_name: Option<String>,
    #[serde(default = "default_text_input_name")]
    pub text_input_name: String,
    #[serde(default = "default_audio_output_name")]
    pub audio_output_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossVoice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub speaker_id: Option<i64>,
}

fn default_sample_rate() -> u32 {
    DEFAULT_SAMPLE_RATE
}
fn default_channels() -> u16 {
    1
}
fn default_text_input_name() -> String {
    "input_ids".to_string()
}
fn default_audio_output_name() -> String {
    "audio".to_string()
}

/// Reasons why MOSS failed to load. Reported as diagnostics and translated
/// to HTTP responses by the caller.
#[derive(Debug)]
pub enum LoadError {
    RuntimeDylibMissing { searched: Vec<PathBuf>, cause: String },
    RuntimeInit { cause: String },
    ModelMissing { path: PathBuf },
    TokenizerMissing { path: PathBuf },
    TokenizerInvalid(String),
    ConfigInvalid(String),
    ModelLoad(String),
}

impl LoadError {
    pub fn as_diagnostic(&self) -> EngineDiagnostic {
        let (code, message, hint) = match self {
            LoadError::RuntimeDylibMissing { searched, cause } => (
                "onnx.runtime_missing",
                format!("ONNX Runtime を読み込めません: {cause}"),
                Some(format!(
                    "onnxruntime.dll / libonnxruntime.so を以下のいずれかに配置してください: {}",
                    searched
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )),
            ),
            LoadError::RuntimeInit { cause } => (
                "onnx.runtime_init_failed",
                format!("ONNX Runtime の初期化に失敗しました: {cause}"),
                Some("ORTバージョンとハードウェア要件を確認してください。".to_string()),
            ),
            LoadError::ModelMissing { path } => (
                "model.missing",
                format!("model.onnx が見つかりません: {}", path.display()),
                Some("設定画面でモデルディレクトリを指定するか、モデルを配置してください。".to_string()),
            ),
            LoadError::TokenizerMissing { path } => (
                "tokenizer.missing",
                format!("tokenizer.json が見つかりません: {}", path.display()),
                Some("MOSS-TTS-Nano 互換の tokenizer.json をモデルと同じディレクトリに配置してください。".to_string()),
            ),
            LoadError::TokenizerInvalid(message) => (
                "tokenizer.invalid",
                format!("tokenizer.json の内容が不正です: {message}"),
                None,
            ),
            LoadError::ConfigInvalid(message) => (
                "config.invalid",
                format!("config.json の内容が不正です: {message}"),
                None,
            ),
            LoadError::ModelLoad(message) => (
                "model.load_failed",
                format!("モデルのロードに失敗しました: {message}"),
                Some("ファイル破損やアーキテクチャ不一致の可能性があります。".to_string()),
            ),
        };
        EngineDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: code.to_string(),
            message,
            hint,
        }
    }
}

pub struct LoadOutcome {
    pub engine: Option<MossOnnxEngine>,
    pub diagnostic: Option<EngineDiagnostic>,
}

/// Attempt to load the MOSS-TTS-Nano ONNX engine from the given model directory.
/// Returns `LoadOutcome::engine = None` with a diagnostic when anything is
/// missing or malformed, so the caller can fall back to the test-tone engine
/// while still reporting the problem via `/health`.
pub fn try_load(
    model_dir: Option<&Path>,
    ort_dylib_path: Option<&Path>,
    cpu_threads: u16,
) -> LoadOutcome {
    let Some(model_dir) = model_dir else {
        return LoadOutcome {
            engine: None,
            diagnostic: Some(EngineDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "model.dir_unset".to_string(),
                message: "モデルディレクトリが指定されていません。".to_string(),
                hint: Some("`--model-dir` または設定画面でパスを指定してください。".to_string()),
            }),
        };
    };

    if let Err(error) = configure_ort_dylib(ort_dylib_path, model_dir) {
        return LoadOutcome {
            engine: None,
            diagnostic: Some(error.as_diagnostic()),
        };
    }

    let model_path = model_dir.join("model.onnx");
    if !model_path.exists() {
        return LoadOutcome {
            engine: None,
            diagnostic: Some(LoadError::ModelMissing { path: model_path }.as_diagnostic()),
        };
    }
    let tokenizer_path = model_dir.join("tokenizer.json");
    if !tokenizer_path.exists() {
        return LoadOutcome {
            engine: None,
            diagnostic: Some(
                LoadError::TokenizerMissing {
                    path: tokenizer_path,
                }
                .as_diagnostic(),
            ),
        };
    }
    let config_path = model_dir.join("config.json");
    let config = if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(raw) => match serde_json::from_str::<MossConfig>(&raw) {
                Ok(value) => value,
                Err(error) => {
                    return LoadOutcome {
                        engine: None,
                        diagnostic: Some(
                            LoadError::ConfigInvalid(error.to_string()).as_diagnostic(),
                        ),
                    }
                }
            },
            Err(error) => {
                return LoadOutcome {
                    engine: None,
                    diagnostic: Some(LoadError::ConfigInvalid(error.to_string()).as_diagnostic()),
                }
            }
        }
    } else {
        MossConfig::default()
    };

    let tokenizer = match Tokenizer::from_path(&tokenizer_path) {
        Ok(tok) => tok,
        Err(message) => {
            return LoadOutcome {
                engine: None,
                diagnostic: Some(LoadError::TokenizerInvalid(message).as_diagnostic()),
            };
        }
    };

    let session_builder = match Session::builder() {
        Ok(builder) => builder,
        Err(error) => {
            return LoadOutcome {
                engine: None,
                diagnostic: Some(
                    LoadError::RuntimeInit {
                        cause: error.to_string(),
                    }
                    .as_diagnostic(),
                ),
            };
        }
    };

    let session_builder = session_builder
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap_or_else(|_| Session::builder().unwrap())
        .with_intra_threads(cpu_threads.max(1) as usize)
        .unwrap_or_else(|_| Session::builder().unwrap());

    let session = match session_builder.commit_from_file(&model_path) {
        Ok(session) => session,
        Err(error) => {
            return LoadOutcome {
                engine: None,
                diagnostic: Some(LoadError::ModelLoad(error.to_string()).as_diagnostic()),
            };
        }
    };

    let ort_version = env!("CARGO_PKG_VERSION").to_string();

    LoadOutcome {
        engine: Some(MossOnnxEngine {
            session: Mutex::new(session),
            tokenizer,
            config,
            model_path,
            ort_version,
        }),
        diagnostic: None,
    }
}

fn configure_ort_dylib(explicit: Option<&Path>, model_dir: &Path) -> Result<(), LoadError> {
    let mut searched = Vec::new();
    if let Some(path) = explicit {
        searched.push(path.to_path_buf());
        if path.exists() {
            std::env::set_var("ORT_DYLIB_PATH", path);
            return Ok(());
        }
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let candidates: Vec<PathBuf> = {
        let mut list = Vec::new();
        if let Some(dir) = exe_dir.as_ref() {
            #[cfg(target_os = "windows")]
            list.push(dir.join("onnxruntime.dll"));
            #[cfg(all(unix, not(target_os = "macos")))]
            list.push(dir.join("libonnxruntime.so"));
            #[cfg(target_os = "macos")]
            list.push(dir.join("libonnxruntime.dylib"));
        }
        #[cfg(target_os = "windows")]
        list.push(model_dir.join("onnxruntime.dll"));
        #[cfg(all(unix, not(target_os = "macos")))]
        list.push(model_dir.join("libonnxruntime.so"));
        #[cfg(target_os = "macos")]
        list.push(model_dir.join("libonnxruntime.dylib"));
        list
    };

    for candidate in &candidates {
        searched.push(candidate.clone());
        if candidate.exists() {
            std::env::set_var("ORT_DYLIB_PATH", candidate);
            return Ok(());
        }
    }

    // If we didn't find anything, ORT will try the default dynamic loader
    // search paths. If that fails too, ort::Session::builder() returns an
    // error that we surface through LoadError::RuntimeInit. We still record
    // the searched paths in case the user asks why.
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        return Err(LoadError::RuntimeDylibMissing {
            searched,
            cause: "ORT_DYLIB_PATH未設定、既定パスにも ONNX Runtime が見つかりません。"
                .to_string(),
        });
    }
    Ok(())
}

impl TtsEngine for MossOnnxEngine {
    fn id(&self) -> &'static str {
        "moss-tts-nano-onnx"
    }

    fn name(&self) -> &'static str {
        "MOSS-TTS-Nano (ONNX Runtime)"
    }

    fn diagnostics(&self) -> Vec<EngineDiagnostic> {
        vec![EngineDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: "engine.ready".to_string(),
            message: format!(
                "MOSS-TTS-Nano ONNX 読み込み済 · ORT {} · model={}",
                self.ort_version,
                self.model_path.display()
            ),
            hint: None,
        }]
    }

    fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        if self.config.voices.is_empty() {
            vec![VoiceInfo {
                id: "default".to_string(),
                name: "Default".to_string(),
            }]
        } else {
            self.config
                .voices
                .iter()
                .map(|voice| VoiceInfo {
                    id: voice.id.clone(),
                    name: voice.name.clone(),
                })
                .collect()
        }
    }

    fn synthesize(&self, request: SynthInput) -> Result<SynthResult, SynthError> {
        let ids = self.tokenizer.encode(&request.text)?;
        let seq_len = ids.len();
        if seq_len == 0 {
            return Err(SynthError::EmptyText);
        }
        let input_array: Array2<i64> = Array2::from_shape_vec((1, seq_len), ids.clone())
            .map_err(|error| SynthError::BadShape(error.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|error| SynthError::Inference(format!("session lock: {error}")))?;

        let input_value = Value::from_array(input_array)
            .map_err(|error| SynthError::Inference(error.to_string()))?;

        let mut inputs: Vec<(&str, SessionInputValue<'_>)> = Vec::new();
        inputs.push((
            self.config.text_input_name.as_str(),
            SessionInputValue::from(input_value),
        ));

        if let Some(name) = self.config.speaker_input_name.as_deref() {
            let speaker_id = resolve_speaker_id(&self.config.voices, request.voice.as_deref());
            let speaker_array: Array1<i64> = Array1::from(vec![speaker_id]);
            let speaker_value = Value::from_array(speaker_array)
                .map_err(|error| SynthError::Inference(error.to_string()))?;
            inputs.push((name, SessionInputValue::from(speaker_value)));
        }

        if let Some(name) = self.config.seed_input_name.as_deref() {
            let seed_array: Array1<i64> = Array1::from(vec![request.seed.unwrap_or(0) as i64]);
            let seed_value = Value::from_array(seed_array)
                .map_err(|error| SynthError::Inference(error.to_string()))?;
            inputs.push((name, SessionInputValue::from(seed_value)));
        }

        let outputs = session
            .run(inputs)
            .map_err(|error| SynthError::Inference(error.to_string()))?;

        let audio = outputs
            .get(self.config.audio_output_name.as_str())
            .ok_or_else(|| {
                SynthError::Inference(format!(
                    "モデル出力 `{}` が見つかりません",
                    self.config.audio_output_name
                ))
            })?;
        let (shape, view) = audio
            .try_extract_tensor::<f32>()
            .map_err(|error| SynthError::Inference(error.to_string()))?;
        let flat: Vec<f32> = view.to_vec();
        let samples = flat
            .iter()
            .map(|&value| {
                let clamped = value.clamp(-1.0, 1.0);
                (clamped * i16::MAX as f32).round() as i16
            })
            .collect();

        let expected = shape.iter().product::<i64>();
        if expected as usize != flat.len() {
            return Err(SynthError::BadShape(format!(
                "audio shape {:?} と要素数 {} が一致しません",
                shape,
                flat.len()
            )));
        }

        Ok(SynthResult {
            samples,
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
        })
    }
}

fn resolve_speaker_id(voices: &[MossVoice], requested: Option<&str>) -> i64 {
    let Some(requested) = requested else {
        return voices
            .iter()
            .find_map(|voice| voice.speaker_id)
            .unwrap_or(0);
    };
    voices
        .iter()
        .find(|voice| voice.id == requested)
        .and_then(|voice| voice.speaker_id)
        .unwrap_or(0)
}

impl Default for MossConfig {
    fn default() -> Self {
        Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: 1,
            voices: Vec::new(),
            speaker_input_name: None,
            seed_input_name: None,
            text_input_name: default_text_input_name(),
            audio_output_name: default_audio_output_name(),
        }
    }
}

// Compile-time reference to keep `ArrayD` / `IxDyn` imports honest if the
// synthesize body is extended to handle higher-rank tensors later.
#[allow(dead_code)]
fn _touch_imports() {
    let _: Option<ArrayD<f32>> = None;
    let _: Option<IxDyn> = None;
}
