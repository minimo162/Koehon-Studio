//! MOSS-TTS-Nano multi-stage ONNX inference engine.
//!
//! This module loads the 5-stage autoregressive pipeline (prefill, decode_step,
//! local_fixed_sampled_frame, local_cached_step, local_decoder) along with the
//! manifest + tokenizer.model. The actual autoregressive generation loop is
//! **not implemented yet** — see `docs/MOSS_PIPELINE.md` for the spec.
//!
//! When loaded successfully this engine:
//! - Resolves all 5 ONNX files and their external-data shards
//! - Opens an `ort::Session` for each, with the external-data directory wired up
//! - Parses `tts_browser_onnx_meta.json` and `browser_poc_manifest.json`
//! - Reports the 18 builtin voices to `/health` and UI
//! - `synthesize()` returns a clear `SynthError::Inference` explaining which
//!   step of the pipeline still needs wiring

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ort::session::{builder::GraphOptimizationLevel, Session};
use serde::Deserialize;

use super::{
    DiagnosticSeverity, EngineDiagnostic, SynthError, SynthInput, SynthResult, TtsEngine,
    VoiceInfo,
};

/// The 5 ONNX graphs in the MOSS-TTS-Nano pipeline, keyed by their role in the manifest.
const REQUIRED_ONNX_KEYS: [&str; 5] = [
    "prefill",
    "decode_step",
    "local_decoder",
    "local_cached_step",
    "local_fixed_sampled_frame",
];

pub struct MossTtsNanoEngine {
    model_dir: PathBuf,
    meta: MossOnnxMeta,
    manifest: Option<MossManifest>,
    tokenizer_path: PathBuf,
    sessions: HashMap<String, Mutex<Session>>,
    voices: Vec<VoiceInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossOnnxMeta {
    pub format_version: u32,
    pub files: HashMap<String, String>,
    pub external_data_files: HashMap<String, Vec<String>>,
    pub model_config: MossModelConfig,
    pub onnx: MossOnnxIoMeta,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossModelConfig {
    pub n_vq: u32,
    pub row_width: u32,
    pub hidden_size: u32,
    pub global_layers: u32,
    pub global_heads: u32,
    pub head_dim: u32,
    pub vocab_size: u32,
    pub audio_pad_token_id: u32,
    pub pad_token_id: u32,
    pub im_start_token_id: u32,
    pub im_end_token_id: u32,
    pub audio_start_token_id: u32,
    pub audio_end_token_id: u32,
    pub audio_user_slot_token_id: u32,
    pub audio_assistant_slot_token_id: u32,
    #[serde(default)]
    pub audio_codebook_sizes: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossOnnxIoMeta {
    pub opset: u32,
    #[serde(default)]
    pub prefill_output_names: Vec<String>,
    #[serde(default)]
    pub decode_input_names: Vec<String>,
    #[serde(default)]
    pub decode_output_names: Vec<String>,
    #[serde(default)]
    pub local_cached_input_names: Vec<String>,
    #[serde(default)]
    pub local_cached_output_names: Vec<String>,
    #[serde(default)]
    pub local_fixed_sampled_frame_input_names: Vec<String>,
    #[serde(default)]
    pub local_fixed_sampled_frame_output_names: Vec<String>,
    #[serde(default)]
    pub fixed_sampled_frame_constants: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossManifest {
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub tts_config: serde_json::Value,
    #[serde(default)]
    pub prompt_templates: MossPromptTemplates,
    #[serde(default)]
    pub generation_defaults: MossGenerationDefaults,
    #[serde(default)]
    pub builtin_voices: Vec<MossBuiltinVoice>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MossPromptTemplates {
    #[serde(default)]
    pub user_prompt_prefix_token_ids: Vec<i64>,
    #[serde(default)]
    pub user_prompt_after_reference_token_ids: Vec<i64>,
    #[serde(default)]
    pub assistant_prompt_prefix_token_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MossGenerationDefaults {
    #[serde(default)]
    pub max_new_frames: u32,
    #[serde(default)]
    pub do_sample: bool,
    #[serde(default)]
    pub text_temperature: f32,
    #[serde(default)]
    pub text_top_p: f32,
    #[serde(default)]
    pub text_top_k: u32,
    #[serde(default)]
    pub audio_temperature: f32,
    #[serde(default)]
    pub audio_top_p: f32,
    #[serde(default)]
    pub audio_top_k: u32,
    #[serde(default)]
    pub audio_repetition_penalty: f32,
    #[serde(default)]
    pub sample_mode: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MossBuiltinVoice {
    pub voice: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub audio_file: Option<String>,
    /// Pre-computed audio codes to prepend as voice reference, shape
    /// `[N, n_vq=16]`. Skipped during deserialization by default because it's
    /// large; kept as untyped JSON until we actually need it.
    #[serde(default)]
    pub prompt_audio_codes: serde_json::Value,
}

#[derive(Debug)]
pub enum MossLoadError {
    MetaMissing(PathBuf),
    MetaInvalid(String),
    ManifestInvalid(String),
    TokenizerMissing(PathBuf),
    OnnxMissing { key: String, path: PathBuf },
    ExternalDataMissing { onnx: String, missing: Vec<PathBuf> },
    RuntimeInit { cause: String },
    SessionLoad { key: String, cause: String },
}

impl MossLoadError {
    pub fn as_diagnostic(&self) -> EngineDiagnostic {
        let (code, message, hint) = match self {
            MossLoadError::MetaMissing(path) => (
                "moss.meta_missing",
                format!("tts_browser_onnx_meta.json が見つかりません: {}", path.display()),
                Some("MOSS-TTS-Nano のダウンロードが完了しているか確認してください。".to_string()),
            ),
            MossLoadError::MetaInvalid(msg) => (
                "moss.meta_invalid",
                format!("tts_browser_onnx_meta.json の JSON が不正です: {msg}"),
                None,
            ),
            MossLoadError::ManifestInvalid(msg) => (
                "moss.manifest_invalid",
                format!("browser_poc_manifest.json の JSON が不正です: {msg}"),
                None,
            ),
            MossLoadError::TokenizerMissing(path) => (
                "moss.tokenizer_missing",
                format!("tokenizer.model が見つかりません: {}", path.display()),
                None,
            ),
            MossLoadError::OnnxMissing { key, path } => (
                "moss.onnx_missing",
                format!("{key} ONNX が見つかりません: {}", path.display()),
                Some("モデルダウンロードが途中で止まっていないか確認してください。".to_string()),
            ),
            MossLoadError::ExternalDataMissing { onnx, missing } => (
                "moss.external_data_missing",
                format!(
                    "{onnx} の external-data ファイルが不足しています: {}",
                    missing
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Some(".data ファイル (数百MB) のダウンロードが完了しているか確認してください。".to_string()),
            ),
            MossLoadError::RuntimeInit { cause } => (
                "onnx.runtime_init_failed",
                format!("ONNX Runtime の初期化に失敗しました: {cause}"),
                None,
            ),
            MossLoadError::SessionLoad { key, cause } => (
                "moss.session_load_failed",
                format!("{key} のセッション作成に失敗しました: {cause}"),
                None,
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

pub struct MossLoadOutcome {
    pub engine: Option<MossTtsNanoEngine>,
    pub diagnostics: Vec<EngineDiagnostic>,
}

/// Detect whether `model_dir` contains the MOSS-TTS-Nano layout (meta JSON).
pub fn is_moss_layout(model_dir: &Path) -> bool {
    model_dir.join("tts_browser_onnx_meta.json").exists()
}

pub fn try_load(model_dir: &Path, cpu_threads: u16) -> MossLoadOutcome {
    let mut diagnostics = Vec::new();

    let meta_path = model_dir.join("tts_browser_onnx_meta.json");
    let meta: MossOnnxMeta = match fs::read_to_string(&meta_path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(meta) => meta,
            Err(err) => {
                diagnostics.push(MossLoadError::MetaInvalid(err.to_string()).as_diagnostic());
                return MossLoadOutcome {
                    engine: None,
                    diagnostics,
                };
            }
        },
        Err(_) => {
            diagnostics.push(MossLoadError::MetaMissing(meta_path).as_diagnostic());
            return MossLoadOutcome {
                engine: None,
                diagnostics,
            };
        }
    };

    // Ensure we know how to handle this meta's format.
    if meta.format_version != 1 {
        diagnostics.push(EngineDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "moss.meta_version_unknown".to_string(),
            message: format!(
                "tts_browser_onnx_meta.json の format_version={} に初期対応しています (想定: 1)",
                meta.format_version
            ),
            hint: None,
        });
    }

    // Verify all 5 ONNX files + their external-data files exist.
    for key in REQUIRED_ONNX_KEYS {
        let Some(filename) = meta.files.get(key) else {
            diagnostics.push(EngineDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "moss.meta_missing_key".to_string(),
                message: format!(
                    "tts_browser_onnx_meta.json の files に \"{key}\" が定義されていません。"
                ),
                hint: None,
            });
            return MossLoadOutcome {
                engine: None,
                diagnostics,
            };
        };
        let onnx_path = model_dir.join(filename);
        if !onnx_path.exists() {
            diagnostics.push(
                MossLoadError::OnnxMissing {
                    key: key.to_string(),
                    path: onnx_path,
                }
                .as_diagnostic(),
            );
            return MossLoadOutcome {
                engine: None,
                diagnostics,
            };
        }
        if let Some(externals) = meta.external_data_files.get(filename) {
            let missing: Vec<PathBuf> = externals
                .iter()
                .map(|name| model_dir.join(name))
                .filter(|p| !p.exists())
                .collect();
            if !missing.is_empty() {
                diagnostics.push(
                    MossLoadError::ExternalDataMissing {
                        onnx: filename.clone(),
                        missing,
                    }
                    .as_diagnostic(),
                );
                return MossLoadOutcome {
                    engine: None,
                    diagnostics,
                };
            }
        }
    }

    // tokenizer.model (SentencePiece binary) must be present.
    let tokenizer_path = model_dir.join("tokenizer.model");
    if !tokenizer_path.exists() {
        diagnostics.push(MossLoadError::TokenizerMissing(tokenizer_path).as_diagnostic());
        return MossLoadOutcome {
            engine: None,
            diagnostics,
        };
    }

    // browser_poc_manifest.json is optional. If it exists we expose voices.
    let manifest_path = model_dir.join("browser_poc_manifest.json");
    let manifest: Option<MossManifest> = if manifest_path.exists() {
        match fs::read_to_string(&manifest_path) {
            Ok(raw) => match serde_json::from_str::<MossManifest>(&raw) {
                Ok(m) => Some(m),
                Err(err) => {
                    diagnostics.push(MossLoadError::ManifestInvalid(err.to_string()).as_diagnostic());
                    None
                }
            },
            Err(_) => None,
        }
    } else {
        diagnostics.push(EngineDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "moss.manifest_missing".to_string(),
            message:
                "browser_poc_manifest.json が見つかりません。voice プロンプトが無効になります。"
                    .to_string(),
            hint: None,
        });
        None
    };

    // Open an ort::Session for each ONNX. External data is auto-resolved from
    // the same directory, which ort locates via the ONNX model's
    // `tensor_external_data.location` entries.
    let mut sessions: HashMap<String, Mutex<Session>> = HashMap::new();
    for key in REQUIRED_ONNX_KEYS {
        let filename = meta.files.get(key).expect("presence checked above").clone();
        let onnx_path = model_dir.join(&filename);

        let builder = match Session::builder() {
            Ok(b) => b,
            Err(err) => {
                diagnostics.push(
                    MossLoadError::RuntimeInit {
                        cause: err.to_string(),
                    }
                    .as_diagnostic(),
                );
                return MossLoadOutcome {
                    engine: None,
                    diagnostics,
                };
            }
        };

        let builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .unwrap_or_else(|_| Session::builder().unwrap())
            .with_intra_threads(cpu_threads.max(1) as usize)
            .unwrap_or_else(|_| Session::builder().unwrap());

        let session = match builder.commit_from_file(&onnx_path) {
            Ok(session) => session,
            Err(err) => {
                diagnostics.push(
                    MossLoadError::SessionLoad {
                        key: key.to_string(),
                        cause: err.to_string(),
                    }
                    .as_diagnostic(),
                );
                return MossLoadOutcome {
                    engine: None,
                    diagnostics,
                };
            }
        };

        sessions.insert(key.to_string(), Mutex::new(session));
    }

    let voices = build_voice_list(manifest.as_ref());

    let engine = MossTtsNanoEngine {
        model_dir: model_dir.to_path_buf(),
        meta,
        manifest,
        tokenizer_path,
        sessions,
        voices,
    };
    MossLoadOutcome {
        engine: Some(engine),
        diagnostics,
    }
}

fn build_voice_list(manifest: Option<&MossManifest>) -> Vec<VoiceInfo> {
    let Some(manifest) = manifest else {
        return vec![VoiceInfo {
            id: "default".to_string(),
            name: "Default (manifest unavailable)".to_string(),
        }];
    };
    if manifest.builtin_voices.is_empty() {
        return vec![VoiceInfo {
            id: "default".to_string(),
            name: "Default (no builtin voices)".to_string(),
        }];
    }
    manifest
        .builtin_voices
        .iter()
        .map(|v| VoiceInfo {
            id: v.voice.clone(),
            name: match (&v.display_name, &v.group) {
                (Some(display), Some(group)) => format!("{display} · {group}"),
                (Some(display), None) => display.clone(),
                (None, Some(group)) => format!("{} · {}", v.voice, group),
                (None, None) => v.voice.clone(),
            },
        })
        .collect()
}

impl TtsEngine for MossTtsNanoEngine {
    fn id(&self) -> &'static str {
        "moss-tts-nano-onnx"
    }

    fn name(&self) -> &'static str {
        "MOSS-TTS-Nano 100M (ONNX, 5-stage pipeline)"
    }

    fn diagnostics(&self) -> Vec<EngineDiagnostic> {
        let mut items = vec![EngineDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: "moss.ready_but_stubbed".to_string(),
            message: format!(
                "MOSS-TTS-Nano の 5 ONNX Session を読み込み済み。詳細は docs/MOSS_PIPELINE.md 参照。{}",
                self.model_dir.display()
            ),
            hint: Some(
                "autoregressive 生成ループは未実装のため、synthesize は失敗します。実装状況は docs/MOSS_PIPELINE.md のチェックリストで追跡しています。".to_string(),
            ),
        }];
        if let Some(manifest) = &self.manifest {
            items.push(EngineDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "moss.manifest_loaded".to_string(),
                message: format!(
                    "builtin voices: {} · prompt_templates: {} + {} + {} tokens",
                    manifest.builtin_voices.len(),
                    manifest.prompt_templates.user_prompt_prefix_token_ids.len(),
                    manifest
                        .prompt_templates
                        .user_prompt_after_reference_token_ids
                        .len(),
                    manifest
                        .prompt_templates
                        .assistant_prompt_prefix_token_ids
                        .len(),
                ),
                hint: None,
            });
        }
        items.push(EngineDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: "moss.tokenizer_path".to_string(),
            message: format!(
                "tokenizer.model = {} (SentencePiece integration pending)",
                self.tokenizer_path.display()
            ),
            hint: None,
        });
        items
    }

    fn sample_rate(&self) -> u32 {
        48_000
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        self.voices.clone()
    }

    fn synthesize(&self, _request: SynthInput) -> Result<SynthResult, SynthError> {
        Err(SynthError::Inference(
            "MOSS-TTS-Nano autoregressive 推論ループは未実装です。実装進捗は docs/MOSS_PIPELINE.md を参照してください。現在はテストトーンエンジンに自動フォールバックしません (明示的なエラーを返す設計)。".to_string(),
        ))
    }
}

impl MossTtsNanoEngine {
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }
    pub fn meta(&self) -> &MossOnnxMeta {
        &self.meta
    }
    pub fn manifest(&self) -> Option<&MossManifest> {
        self.manifest.as_ref()
    }
    pub fn session_keys(&self) -> Vec<&str> {
        self.sessions.keys().map(String::as_str).collect()
    }
}
