use serde::Serialize;

pub mod moss_onnx;
pub mod moss_tts_nano;
pub mod test_tone;
pub mod tokenizer;

/// Output audio from a single synthesize request.
#[derive(Debug, Clone)]
pub struct SynthResult {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Non-fatal status information about the engine, surfaced via `/health`.
#[derive(Debug, Clone, Serialize)]
pub struct EngineDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A recoverable synthesize error. The sidecar translates this into an
/// HTTP 500-family response with a human readable Japanese message.
#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    #[error("TTSエンジンが初期化されていません")]
    NotReady,
    #[error("入力テキストが空です")]
    EmptyText,
    #[error("モデル推論に失敗しました: {0}")]
    Inference(String),
    #[error("入力のトークン化に失敗しました: {0}")]
    Tokenize(String),
    #[error("入出力テンソルの形式が不正です: {0}")]
    BadShape(String),
}

pub trait TtsEngine: Send + Sync {
    /// Engine identifier, surfaced by `/health`.
    fn id(&self) -> &'static str;
    /// Human readable engine label.
    fn name(&self) -> &'static str;
    /// Non-fatal status items. Empty list means "everything is fine".
    fn diagnostics(&self) -> Vec<EngineDiagnostic>;
    /// Sample rate of produced audio.
    fn sample_rate(&self) -> u32;
    /// Available voice identifiers.
    fn voices(&self) -> Vec<VoiceInfo>;
    /// Render a single utterance. Blocking.
    fn synthesize(&self, request: SynthInput) -> Result<SynthResult, SynthError>;
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SynthInput {
    pub text: String,
    pub voice: Option<String>,
    pub seed: Option<u64>,
}
