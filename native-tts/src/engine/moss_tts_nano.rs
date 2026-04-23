//! MOSS-TTS-Nano multi-stage ONNX inference engine.
//!
//! Pipeline (see `docs/MOSS_PIPELINE.md` for the full spec):
//!
//!   text → SentencePiece tokens → prompt build ([1, seq, 17]) →
//!   prefill.onnx → global_hidden + 12-layer KV cache →
//!   loop: local_fixed_sampled_frame (sampling) + decode_step (KV update) →
//!   audio_codes [frames, 16] → audio_tokenizer_decode_full.onnx → 48kHz 2ch PCM

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ndarray::{Array, Array1, Array2, Array3, Array4, ArrayD, Axis, IxDyn};
use ort::session::{builder::GraphOptimizationLevel, Session, SessionInputValue};
use ort::value::Value;
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use sentencepiece::SentencePieceProcessor;
use serde::Deserialize;

use super::{
    DiagnosticSeverity, EngineDiagnostic, SynthError, SynthInput, SynthResult, TtsEngine,
    VoiceInfo,
};

// We load only the three ONNX graphs the current synthesize loop actually
// calls. `local_decoder` and `local_cached_step` are part of the MOSS
// release but unused by the `sample_mode = "fixed"` path — loading them
// multiplied the resident weight set on low-memory hosts and caused OOM
// aborts during inference.
const REQUIRED_ONNX_KEYS: [&str; 3] = [
    "prefill",
    "decode_step",
    "local_fixed_sampled_frame",
];

const GLOBAL_LAYERS: usize = 12;
const HIDDEN_SIZE: usize = 768;
const N_VQ: usize = 16;
const AUDIO_CODEBOOK_SIZE: usize = 1024;
const ROW_WIDTH: usize = 17; // 1 text + 16 audio
const CODEC_SAMPLE_RATE: u32 = 48_000;
const CODEC_CHANNELS: u16 = 2;

pub struct MossTtsNanoEngine {
    model_dir: PathBuf,
    meta: MossOnnxMeta,
    manifest: Option<MossManifest>,
    tokenizer: SentencePieceProcessor,
    tokenizer_path: PathBuf,
    sessions: HashMap<String, Mutex<Session>>,
    codec: Option<Mutex<Session>>,
    codec_dir: Option<PathBuf>,
    voices: Vec<VoiceInfo>,
    extra_diagnostics: Vec<EngineDiagnostic>,
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
    /// `[N, 16]` audio codes to prepend as voice conditioning.
    #[serde(default)]
    pub prompt_audio_codes: Vec<Vec<i32>>,
}

#[derive(Debug)]
pub enum MossLoadError {
    MetaMissing(PathBuf),
    MetaInvalid(String),
    ManifestInvalid(String),
    TokenizerMissing(PathBuf),
    TokenizerLoad(String),
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
            MossLoadError::TokenizerLoad(msg) => (
                "moss.tokenizer_load",
                format!("tokenizer.model の読み込みに失敗しました: {msg}"),
                None,
            ),
            MossLoadError::OnnxMissing { key, path } => (
                "moss.onnx_missing",
                format!("{key} ONNX が見つかりません: {}", path.display()),
                Some("モデルダウンロードが完了しているか確認してください。".to_string()),
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

pub fn is_moss_layout(model_dir: &Path) -> bool {
    model_dir.join("tts_browser_onnx_meta.json").exists()
}

pub fn try_load(
    model_dir: &Path,
    codec_dir: Option<&Path>,
    cpu_threads: u16,
) -> MossLoadOutcome {
    let mut diagnostics: Vec<EngineDiagnostic> = Vec::new();

    let meta_path = model_dir.join("tts_browser_onnx_meta.json");
    let meta: MossOnnxMeta = match fs::read_to_string(&meta_path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(meta) => meta,
            Err(err) => {
                diagnostics.push(MossLoadError::MetaInvalid(err.to_string()).as_diagnostic());
                return MossLoadOutcome { engine: None, diagnostics };
            }
        },
        Err(_) => {
            diagnostics.push(MossLoadError::MetaMissing(meta_path).as_diagnostic());
            return MossLoadOutcome { engine: None, diagnostics };
        }
    };

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
            return MossLoadOutcome { engine: None, diagnostics };
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
            return MossLoadOutcome { engine: None, diagnostics };
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
                return MossLoadOutcome { engine: None, diagnostics };
            }
        }
    }

    let tokenizer_path = model_dir.join("tokenizer.model");
    if !tokenizer_path.exists() {
        diagnostics.push(MossLoadError::TokenizerMissing(tokenizer_path).as_diagnostic());
        return MossLoadOutcome { engine: None, diagnostics };
    }
    let tokenizer = match SentencePieceProcessor::open(&tokenizer_path) {
        Ok(processor) => processor,
        Err(err) => {
            diagnostics.push(MossLoadError::TokenizerLoad(err.to_string()).as_diagnostic());
            return MossLoadOutcome { engine: None, diagnostics };
        }
    };

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

    let mut sessions: HashMap<String, Mutex<Session>> = HashMap::new();
    for key in REQUIRED_ONNX_KEYS {
        let filename = meta.files.get(key).expect("presence checked above").clone();
        let onnx_path = model_dir.join(&filename);
        match build_session(&onnx_path, cpu_threads) {
            Ok(session) => {
                sessions.insert(key.to_string(), Mutex::new(session));
            }
            Err(err) => {
                diagnostics.push(
                    MossLoadError::SessionLoad {
                        key: key.to_string(),
                        cause: err,
                    }
                    .as_diagnostic(),
                );
                return MossLoadOutcome { engine: None, diagnostics };
            }
        };
    }

    // Attempt to load the companion audio codec decoder. Without it we can
    // still run the LLM but not produce real audio.
    let (codec, codec_dir_used) = locate_and_load_codec(model_dir, codec_dir, cpu_threads, &mut diagnostics);

    let voices = build_voice_list(manifest.as_ref());

    let engine = MossTtsNanoEngine {
        model_dir: model_dir.to_path_buf(),
        meta,
        manifest,
        tokenizer,
        tokenizer_path,
        sessions,
        codec,
        codec_dir: codec_dir_used,
        voices,
        extra_diagnostics: Vec::new(),
    };
    MossLoadOutcome {
        engine: Some(engine),
        diagnostics,
    }
}

fn build_session(onnx_path: &Path, cpu_threads: u16) -> Result<Session, String> {
    // Memory tuning notes:
    // - Level2 (not Level3): Level3 applies aggressive constant-folding that
    //   copies weights out of the mmap-backed external-data file into private
    //   session memory. With 5 sessions sharing moss_tts_global_shared.data
    //   (420MB) this multiplies ~5x and was the main OOM driver.
    // - prepacking(false): same reason — prepacking reorders weights into
    //   private blocks eagerly at load. Leaving it off costs ~10% speed but
    //   saves ~1GB RSS at load time, which matters on the 5.8GB host.
    // - env_allocators(): share the allocator across sessions instead of each
    //   session owning its own arena.
    // - memory_pattern(false): memory pattern planning caches buffer shapes
    //   from the first run; with a KV cache that grows by 1 every frame the
    //   planner keeps expanding without bound.
    // - *_op_spinning(false): reduces idle CPU burn when threads wait.
    let builder = Session::builder().map_err(|e| e.to_string())?;
    let builder = builder
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|e| e.to_string())?
        .with_intra_threads(cpu_threads.max(1) as usize)
        .map_err(|e| e.to_string())?
        .with_memory_pattern(false)
        .map_err(|e| e.to_string())?
        .with_prepacking(false)
        .map_err(|e| e.to_string())?
        .with_env_allocators()
        .map_err(|e| e.to_string())?
        .with_intra_op_spinning(false)
        .map_err(|e| e.to_string())?
        .with_inter_op_spinning(false)
        .map_err(|e| e.to_string())?;
    builder.commit_from_file(onnx_path).map_err(|e| e.to_string())
}

fn locate_and_load_codec(
    model_dir: &Path,
    explicit_codec_dir: Option<&Path>,
    cpu_threads: u16,
    diagnostics: &mut Vec<EngineDiagnostic>,
) -> (Option<Mutex<Session>>, Option<PathBuf>) {
    let candidates: Vec<PathBuf> = {
        let mut list = Vec::new();
        if let Some(dir) = explicit_codec_dir {
            list.push(dir.to_path_buf());
        }
        if let Some(parent) = model_dir.parent() {
            list.push(parent.join("moss-audio-tokenizer"));
        }
        list
    };

    for candidate in candidates {
        let decode_path = candidate.join("moss_audio_tokenizer_decode_full.onnx");
        if !decode_path.exists() {
            continue;
        }
        let data_path = candidate.join("moss_audio_tokenizer_decode_shared.data");
        if !data_path.exists() {
            diagnostics.push(EngineDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "codec.external_data_missing".to_string(),
                message: format!(
                    "{} が見つかりません。音声コーデック推論は失敗します。",
                    data_path.display()
                ),
                hint: None,
            });
            continue;
        }
        match build_session(&decode_path, cpu_threads) {
            Ok(session) => {
                diagnostics.push(EngineDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    code: "codec.loaded".to_string(),
                    message: format!(
                        "MOSS Audio Tokenizer decode session を読み込みました: {}",
                        decode_path.display()
                    ),
                    hint: None,
                });
                return (Some(Mutex::new(session)), Some(candidate));
            }
            Err(err) => {
                diagnostics.push(EngineDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: "codec.load_failed".to_string(),
                    message: format!("Audio codec のロードに失敗しました: {err}"),
                    hint: None,
                });
                return (None, None);
            }
        }
    }

    diagnostics.push(EngineDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "codec.missing".to_string(),
        message: "MOSS Audio Tokenizer が見つかりません (音声トークン → PCM 復号が不可)。"
            .to_string(),
        hint: Some(
            "設定画面から MOSS Audio Tokenizer もダウンロードし、モデルディレクトリと同じ親フォルダの moss-audio-tokenizer/ に配置してください。"
                .to_string(),
        ),
    });
    (None, None)
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
        let mut items = Vec::new();
        items.push(EngineDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: "moss.ready".to_string(),
            message: format!(
                "MOSS-TTS-Nano 読込済 (5 sessions, codec={})",
                if self.codec.is_some() { "yes" } else { "no" }
            ),
            hint: None,
        });
        if let Some(manifest) = &self.manifest {
            items.push(EngineDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "moss.manifest_loaded".to_string(),
                message: format!(
                    "builtin voices: {} · prompt_templates: {} + {} + {} tokens · sample_mode={}",
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
                    manifest.generation_defaults.sample_mode,
                ),
                hint: None,
            });
        }
        if self.codec.is_none() {
            items.push(EngineDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "codec.unavailable".to_string(),
                message:
                    "音声コーデックが未ロードのため、synthesize は失敗します (音声生成トークンまでは成功)。"
                        .to_string(),
                hint: Some(
                    "MOSS Audio Tokenizer を同じ親フォルダに配置するか、--codec-dir で指定してください。"
                        .to_string(),
                ),
            });
        }
        items.extend(self.extra_diagnostics.iter().cloned());
        items
    }

    fn sample_rate(&self) -> u32 {
        CODEC_SAMPLE_RATE
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        self.voices.clone()
    }

    fn synthesize(&self, request: SynthInput) -> Result<SynthResult, SynthError> {
        let text = request.text.trim();
        if text.is_empty() {
            return Err(SynthError::EmptyText);
        }

        let manifest = self
            .manifest
            .as_ref()
            .ok_or_else(|| SynthError::Inference("browser_poc_manifest.json がないためプロンプト構築ができません。".to_string()))?;

        let voice = select_voice(manifest, request.voice.as_deref());
        let text_ids = self
            .tokenizer
            .encode(text)
            .map_err(|e| SynthError::Tokenize(e.to_string()))?
            .into_iter()
            .map(|p| p.id as i64)
            .collect::<Vec<_>>();

        let (input_ids, attention_mask) = build_prompt(
            &manifest.prompt_templates,
            voice,
            &text_ids,
            &self.meta.model_config,
        );
        let seq_len = attention_mask.len();

        // --- Prefill -----------------------------------------------------
        let (global_hidden, mut past_keys, mut past_values) = {
            let mut session = self
                .sessions
                .get("prefill")
                .ok_or_else(|| SynthError::Inference("prefill session missing".to_string()))?
                .lock()
                .map_err(|e| SynthError::Inference(format!("prefill lock: {e}")))?;
            let input_ids_arr: Array3<i32> = Array3::from_shape_vec(
                (1, seq_len, ROW_WIDTH),
                input_ids.into_iter().flatten().collect(),
            )
            .map_err(|e| SynthError::BadShape(e.to_string()))?;
            let mask_arr: Array2<i32> =
                Array2::from_shape_vec((1, seq_len), attention_mask.clone())
                    .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let input_ids_val = Value::from_array(input_ids_arr)
                .map_err(|e| SynthError::Inference(format!("prefill input_ids: {e}")))?;
            let mask_val = Value::from_array(mask_arr)
                .map_err(|e| SynthError::Inference(format!("prefill mask: {e}")))?;
            let inputs: Vec<(&str, SessionInputValue<'_>)> = vec![
                ("input_ids", SessionInputValue::from(input_ids_val)),
                ("attention_mask", SessionInputValue::from(mask_val)),
            ];
            let outputs = session
                .run(inputs)
                .map_err(|e| SynthError::Inference(format!("prefill run: {e}")))?;

            let (_shape, hidden_view) = outputs
                .get("global_hidden")
                .ok_or_else(|| SynthError::Inference("prefill missing global_hidden".to_string()))?
                .try_extract_tensor::<f32>()
                .map_err(|e| SynthError::Inference(format!("prefill global_hidden: {e}")))?;
            let hidden = Array3::from_shape_vec((1, seq_len, HIDDEN_SIZE), hidden_view.to_vec())
                .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let mut keys = Vec::with_capacity(GLOBAL_LAYERS);
            let mut values = Vec::with_capacity(GLOBAL_LAYERS);
            for layer in 0..GLOBAL_LAYERS {
                let key_name = format!("present_key_{layer}");
                let val_name = format!("present_value_{layer}");
                let (k_shape, k_view) = outputs
                    .get(key_name.as_str())
                    .ok_or_else(|| SynthError::Inference(format!("prefill missing {key_name}")))?
                    .try_extract_tensor::<f32>()
                    .map_err(|e| SynthError::Inference(format!("prefill {key_name}: {e}")))?;
                let (v_shape, v_view) = outputs
                    .get(val_name.as_str())
                    .ok_or_else(|| SynthError::Inference(format!("prefill missing {val_name}")))?
                    .try_extract_tensor::<f32>()
                    .map_err(|e| SynthError::Inference(format!("prefill {val_name}: {e}")))?;
                keys.push(kv_from_view(&k_shape, k_view.to_vec())?);
                values.push(kv_from_view(&v_shape, v_view.to_vec())?);
            }
            (hidden, keys, values)
        };

        // --- Frame loop --------------------------------------------------
        let max_frames = if manifest.generation_defaults.max_new_frames == 0 {
            375
        } else {
            manifest.generation_defaults.max_new_frames
        };
        let mut past_valid = vec![seq_len as i32];
        let mut audio_frames: Vec<Vec<i32>> = Vec::new();
        let mut seen_mask = vec![0i32; N_VQ * AUDIO_CODEBOOK_SIZE];
        let seed = request.seed.unwrap_or(0xC0FFEE_u64);
        let mut rng = Pcg64Mcg::seed_from_u64(seed);

        let mut current_hidden = global_hidden;

        for _step in 0..max_frames {
            // Take last-position hidden state [1, HIDDEN_SIZE]
            let last_idx = current_hidden.shape()[1] - 1;
            let last_hidden: Array2<f32> = current_hidden
                .slice(ndarray::s![.., last_idx..last_idx + 1, ..])
                .to_owned()
                .into_shape_with_order((1, HIDDEN_SIZE))
                .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let mask_arr = Array3::from_shape_vec(
                (1, N_VQ, AUDIO_CODEBOOK_SIZE),
                seen_mask.clone(),
            )
            .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let assistant_u = rng.gen::<f32>();
            let assistant_u_arr: Array1<f32> = Array1::from(vec![assistant_u]);
            let audio_u_vec: Vec<f32> = (0..N_VQ).map(|_| rng.gen::<f32>()).collect();
            let audio_u_arr = Array2::from_shape_vec((1, N_VQ), audio_u_vec)
                .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let (should_continue, frame_ids) = {
                let mut session = self
                    .sessions
                    .get("local_fixed_sampled_frame")
                    .ok_or_else(|| SynthError::Inference("sampled_frame session missing".to_string()))?
                    .lock()
                    .map_err(|e| SynthError::Inference(format!("sampled_frame lock: {e}")))?;

                let hidden_val = Value::from_array(last_hidden)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let mask_val = Value::from_array(mask_arr)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let au_val = Value::from_array(assistant_u_arr)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let ru_val = Value::from_array(audio_u_arr)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;

                let inputs: Vec<(&str, SessionInputValue<'_>)> = vec![
                    ("global_hidden", SessionInputValue::from(hidden_val)),
                    ("repetition_seen_mask", SessionInputValue::from(mask_val)),
                    ("assistant_random_u", SessionInputValue::from(au_val)),
                    ("audio_random_u", SessionInputValue::from(ru_val)),
                ];
                let outputs = session
                    .run(inputs)
                    .map_err(|e| SynthError::Inference(format!("sampled_frame run: {e}")))?;

                let (_, cont_view) = outputs
                    .get("should_continue")
                    .ok_or_else(|| SynthError::Inference("missing should_continue".to_string()))?
                    .try_extract_tensor::<i32>()
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let cont_val = *cont_view
                    .first()
                    .ok_or_else(|| SynthError::Inference("empty should_continue".to_string()))?;

                let (_, ids_view) = outputs
                    .get("frame_token_ids")
                    .ok_or_else(|| SynthError::Inference("missing frame_token_ids".to_string()))?
                    .try_extract_tensor::<i32>()
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let ids = ids_view.to_vec();
                if ids.len() != N_VQ {
                    return Err(SynthError::BadShape(format!(
                        "frame_token_ids length {} != {N_VQ}",
                        ids.len()
                    )));
                }
                (cont_val, ids)
            };

            if should_continue == 0 {
                break;
            }

            // update seen mask
            for (channel, token) in frame_ids.iter().enumerate() {
                if *token >= 0 && (*token as usize) < AUDIO_CODEBOOK_SIZE {
                    seen_mask[channel * AUDIO_CODEBOOK_SIZE + (*token as usize)] = 1;
                }
            }

            audio_frames.push(frame_ids.clone());

            // Build next decode_step input: [1, 1, 17] = [assistant_slot, 16 audio tokens]
            let mut next_input = Vec::with_capacity(ROW_WIDTH);
            next_input.push(self.meta.model_config.audio_assistant_slot_token_id as i32);
            next_input.extend_from_slice(&frame_ids);
            let next_arr = Array3::from_shape_vec((1, 1, ROW_WIDTH), next_input)
                .map_err(|e| SynthError::BadShape(e.to_string()))?;

            let past_valid_arr = Array1::from(past_valid.clone());

            // decode_step
            let (new_hidden, new_keys, new_values) = {
                let mut session = self
                    .sessions
                    .get("decode_step")
                    .ok_or_else(|| SynthError::Inference("decode_step session missing".to_string()))?
                    .lock()
                    .map_err(|e| SynthError::Inference(format!("decode_step lock: {e}")))?;

                let input_val = Value::from_array(next_arr)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let past_valid_val = Value::from_array(past_valid_arr)
                    .map_err(|e| SynthError::Inference(e.to_string()))?;

                let mut inputs: Vec<(&str, SessionInputValue<'_>)> = Vec::with_capacity(2 + 2 * GLOBAL_LAYERS);
                inputs.push(("input_ids", SessionInputValue::from(input_val)));
                inputs.push(("past_valid_lengths", SessionInputValue::from(past_valid_val)));
                // Take ownership of past KVs so we can re-upload them.
                let keys_moved = std::mem::take(&mut past_keys);
                let values_moved = std::mem::take(&mut past_values);
                let mut holders: Vec<(String, Value)> = Vec::with_capacity(2 * GLOBAL_LAYERS);
                for (layer, key) in keys_moved.into_iter().enumerate() {
                    let v: Value = Value::from_array(key)
                        .map_err(|e| SynthError::Inference(e.to_string()))?
                        .into();
                    holders.push((format!("past_key_{layer}"), v));
                }
                for (layer, value) in values_moved.into_iter().enumerate() {
                    let v: Value = Value::from_array(value)
                        .map_err(|e| SynthError::Inference(e.to_string()))?
                        .into();
                    holders.push((format!("past_value_{layer}"), v));
                }
                // Re-acquire references with matching lifetimes for the input vec.
                let refs: Vec<(&str, SessionInputValue<'_>)> = holders
                    .iter()
                    .map(|(name, val)| (name.as_str(), SessionInputValue::from(val.view())))
                    .collect();
                inputs.extend(refs);

                let outputs = session
                    .run(inputs)
                    .map_err(|e| SynthError::Inference(format!("decode_step run: {e}")))?;

                let (_, hidden_view) = outputs
                    .get("global_hidden")
                    .ok_or_else(|| SynthError::Inference("decode_step missing global_hidden".to_string()))?
                    .try_extract_tensor::<f32>()
                    .map_err(|e| SynthError::Inference(e.to_string()))?;
                let hidden_vec = hidden_view.to_vec();
                let hidden_len = hidden_vec.len() / HIDDEN_SIZE;
                let hidden = Array3::from_shape_vec((1, hidden_len, HIDDEN_SIZE), hidden_vec)
                    .map_err(|e| SynthError::BadShape(e.to_string()))?;

                let mut keys = Vec::with_capacity(GLOBAL_LAYERS);
                let mut values = Vec::with_capacity(GLOBAL_LAYERS);
                for layer in 0..GLOBAL_LAYERS {
                    let (k_shape, k_view) = outputs
                        .get(format!("present_key_{layer}").as_str())
                        .ok_or_else(|| SynthError::Inference(format!("decode missing present_key_{layer}")))?
                        .try_extract_tensor::<f32>()
                        .map_err(|e| SynthError::Inference(e.to_string()))?;
                    let (v_shape, v_view) = outputs
                        .get(format!("present_value_{layer}").as_str())
                        .ok_or_else(|| SynthError::Inference(format!("decode missing present_value_{layer}")))?
                        .try_extract_tensor::<f32>()
                        .map_err(|e| SynthError::Inference(e.to_string()))?;
                    keys.push(kv_from_view(&k_shape, k_view.to_vec())?);
                    values.push(kv_from_view(&v_shape, v_view.to_vec())?);
                }
                (hidden, keys, values)
            };

            current_hidden = new_hidden;
            past_keys = new_keys;
            past_values = new_values;
            past_valid[0] += 1;
        }

        if audio_frames.is_empty() {
            return Err(SynthError::Inference(
                "生成ループが 1 フレームも進みませんでした (should_continue=0 を初回で受信)".to_string(),
            ));
        }

        // --- Codec decode -----------------------------------------------
        // The codec decoder's transformer self-attention is O(N^2) in the
        // audio-code sequence length; running all ~375 frames at once
        // requested a ~2.3 GB MatMul buffer and OOM-crashed the sidecar on
        // 6 GB hosts. Decode in fixed-size windows instead and concatenate
        // the per-channel PCM output.
        const DECODE_WINDOW_FRAMES: usize = 100;

        let codec_mutex = self.codec.as_ref().ok_or_else(|| {
            SynthError::Inference(
                "Audio codec が読み込まれていないため、音声トークンを波形に変換できません。".to_string(),
            )
        })?;
        let mut codec_session = codec_mutex
            .lock()
            .map_err(|e| SynthError::Inference(format!("codec lock: {e}")))?;

        let total_frames = audio_frames.len();
        let mut left_ch: Vec<f32> = Vec::new();
        let mut right_ch: Vec<f32> = Vec::new();

        for window_start in (0..total_frames).step_by(DECODE_WINDOW_FRAMES) {
            let window_end = (window_start + DECODE_WINDOW_FRAMES).min(total_frames);
            let window_frames = &audio_frames[window_start..window_end];
            let window_count = window_frames.len();
            let flat_codes: Vec<i32> =
                window_frames.iter().flat_map(|f| f.iter().copied()).collect();

            let codes_arr: Array3<i32> =
                Array3::from_shape_vec((1, window_count, N_VQ), flat_codes)
                    .map_err(|e| SynthError::BadShape(e.to_string()))?;
            let lengths_arr: Array1<i32> = Array1::from(vec![window_count as i32]);

            let codes_val = Value::from_array(codes_arr)
                .map_err(|e| SynthError::Inference(e.to_string()))?;
            let lengths_val = Value::from_array(lengths_arr)
                .map_err(|e| SynthError::Inference(e.to_string()))?;

            let inputs: Vec<(&str, SessionInputValue<'_>)> = vec![
                ("audio_codes", SessionInputValue::from(codes_val)),
                ("audio_code_lengths", SessionInputValue::from(lengths_val)),
            ];
            let outputs = codec_session.run(inputs).map_err(|e| {
                SynthError::Inference(format!(
                    "codec decode run (frames {window_start}..{window_end}): {e}"
                ))
            })?;

            let (audio_shape, audio_view) = outputs
                .get("audio")
                .ok_or_else(|| SynthError::Inference("codec missing audio".to_string()))?
                .try_extract_tensor::<f32>()
                .map_err(|e| SynthError::Inference(e.to_string()))?;

            if audio_shape.len() != 3 || audio_shape[1] != CODEC_CHANNELS as i64 {
                return Err(SynthError::BadShape(format!(
                    "codec audio shape {:?} (期待: [1, 2, samples])",
                    audio_shape
                )));
            }
            let samples_per_channel = audio_shape[2] as usize;
            let audio_vec = audio_view.to_vec();

            left_ch.extend_from_slice(&audio_vec[0..samples_per_channel]);
            right_ch
                .extend_from_slice(&audio_vec[samples_per_channel..2 * samples_per_channel]);
        }

        // audio is [2, samples] float in [-1, 1]; interleave LRLR as i16
        let total_samples = left_ch.len();
        let mut pcm: Vec<i16> = Vec::with_capacity(total_samples * 2);
        for i in 0..total_samples {
            pcm.push((left_ch[i].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16);
            pcm.push((right_ch[i].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16);
        }

        Ok(SynthResult {
            samples: pcm,
            sample_rate: CODEC_SAMPLE_RATE,
            channels: CODEC_CHANNELS,
        })
    }
}

fn kv_from_view(shape: &[i64], data: Vec<f32>) -> Result<Array4<f32>, SynthError> {
    if shape.len() != 4 {
        return Err(SynthError::BadShape(format!(
            "KV tensor expected rank 4, got {:?}",
            shape
        )));
    }
    let dims = (
        shape[0] as usize,
        shape[1] as usize,
        shape[2] as usize,
        shape[3] as usize,
    );
    Array4::from_shape_vec(dims, data).map_err(|e| SynthError::BadShape(e.to_string()))
}

fn select_voice<'a>(manifest: &'a MossManifest, requested: Option<&str>) -> Option<&'a MossBuiltinVoice> {
    if manifest.builtin_voices.is_empty() {
        return None;
    }
    if let Some(name) = requested {
        if let Some(found) = manifest
            .builtin_voices
            .iter()
            .find(|v| v.voice.eq_ignore_ascii_case(name))
        {
            return Some(found);
        }
    }
    manifest.builtin_voices.first()
}

/// Build the prefill input tensors.
/// - `input_ids` has shape `[1, seq, 17]` laid out row-major: each row holds
///   `[text_token, audio_code_0, .., audio_code_15]`.
/// - `attention_mask` has shape `[1, seq]` and is all 1s.
fn build_prompt(
    templates: &MossPromptTemplates,
    voice: Option<&MossBuiltinVoice>,
    text_ids: &[i64],
    config: &MossModelConfig,
) -> (Vec<Vec<i32>>, Vec<i32>) {
    let audio_pad = config.audio_pad_token_id as i32;
    let audio_user_slot = config.audio_user_slot_token_id as i32;
    let mut rows: Vec<Vec<i32>> = Vec::new();

    let push_text = |rows: &mut Vec<Vec<i32>>, token: i32| {
        let mut row = Vec::with_capacity(ROW_WIDTH);
        row.push(token);
        for _ in 0..N_VQ {
            row.push(audio_pad);
        }
        rows.push(row);
    };

    for token in &templates.user_prompt_prefix_token_ids {
        push_text(&mut rows, *token as i32);
    }
    if let Some(v) = voice {
        for frame in &v.prompt_audio_codes {
            let mut row = Vec::with_capacity(ROW_WIDTH);
            row.push(audio_user_slot);
            for channel in 0..N_VQ {
                let token = frame.get(channel).copied().unwrap_or(audio_pad);
                row.push(token);
            }
            rows.push(row);
        }
    }
    for token in &templates.user_prompt_after_reference_token_ids {
        push_text(&mut rows, *token as i32);
    }
    for token in text_ids {
        push_text(&mut rows, *token as i32);
    }
    for token in &templates.assistant_prompt_prefix_token_ids {
        push_text(&mut rows, *token as i32);
    }

    let mask = vec![1i32; rows.len()];
    (rows, mask)
}

impl MossTtsNanoEngine {
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }
    pub fn codec_dir(&self) -> Option<&Path> {
        self.codec_dir.as_deref()
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

// Silence unused import warnings when building without certain features later.
#[allow(dead_code)]
fn _touch_types() {
    let _: Option<ArrayD<f32>> = None;
    let _: Option<IxDyn> = None;
    let _: Option<Array<f32, _>> = None::<Array<f32, ndarray::Ix2>>;
    let _: Option<Axis> = None;
}
