use super::{EngineDiagnostic, SynthError, SynthInput, SynthResult, TtsEngine, VoiceInfo};

const SAMPLE_RATE: u32 = 48_000;

/// Fallback engine that produces a short sine tone. Used when the ONNX
/// runtime or model is unavailable so the UI flow stays testable.
pub struct TestToneEngine {
    reason: Option<String>,
}

impl TestToneEngine {
    pub fn new() -> Self {
        Self { reason: None }
    }

    pub fn with_reason(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
        }
    }
}

impl Default for TestToneEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsEngine for TestToneEngine {
    fn id(&self) -> &'static str {
        "koehon-test-tone"
    }

    fn name(&self) -> &'static str {
        "テストトーン (ONNXモデル未配置時のフォールバック)"
    }

    fn diagnostics(&self) -> Vec<EngineDiagnostic> {
        let mut list = vec![EngineDiagnostic {
            severity: super::DiagnosticSeverity::Info,
            code: "engine.test_tone_active".to_string(),
            message: "現在はテストトーンモードで動作しています。実音声は生成されません。".to_string(),
            hint: Some(
                "`--model-dir` で MOSS-TTS-Nano ONNX モデルを指定するとAI音声合成に切り替わります。"
                    .to_string(),
            ),
        }];
        if let Some(reason) = self.reason.as_deref() {
            list.push(EngineDiagnostic {
                severity: super::DiagnosticSeverity::Warning,
                code: "engine.fallback_reason".to_string(),
                message: reason.to_string(),
                hint: None,
            });
        }
        list
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        vec![VoiceInfo {
            id: "default".to_string(),
            name: "Default test tone".to_string(),
        }]
    }

    fn synthesize(&self, request: SynthInput) -> Result<SynthResult, SynthError> {
        if request.text.trim().is_empty() {
            return Err(SynthError::EmptyText);
        }

        let duration_ms = (request.text.chars().count() as u32 * 45).clamp(350, 5_000);
        let frequency = match request.voice.as_deref() {
            Some("default") | None => 440.0,
            Some(_) => 523.25,
        };
        let seed_offset = (request.seed.unwrap_or(0) % 37) as f32;
        let pitch = frequency + seed_offset;

        let total_samples = SAMPLE_RATE as u64 * u64::from(duration_ms) / 1000;
        let total_samples = total_samples as u32;
        let mut samples = Vec::with_capacity(total_samples as usize);
        for index in 0..total_samples {
            let t = index as f32 / SAMPLE_RATE as f32;
            let envelope = if index < 480 {
                index as f32 / 480.0
            } else if total_samples.saturating_sub(index) < 480 {
                total_samples.saturating_sub(index) as f32 / 480.0
            } else {
                1.0
            };
            let sample = (t * pitch * std::f32::consts::TAU).sin() * 0.18 * envelope;
            let pcm = (sample * i16::MAX as f32).round() as i16;
            samples.push(pcm);
        }

        Ok(SynthResult {
            samples,
            sample_rate: SAMPLE_RATE,
            channels: 1,
        })
    }
}
