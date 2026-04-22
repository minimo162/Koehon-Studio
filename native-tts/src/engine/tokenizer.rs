use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

use super::SynthError;

/// Minimal symbol-to-id tokenizer, intentionally kept simple so users can
/// hand-author `tokenizer.json` for a dropped-in ONNX TTS model without
/// depending on heavy phonemizers.
///
/// Expected file shape:
/// ```json
/// {
///   "vocab": { "a": 0, "い": 1, ... },
///   "unknown_id": 0,
///   "bos_id": 1,
///   "eos_id": 2,
///   "pad_id": 3,
///   "mode": "chars"
/// }
/// ```
/// `mode` is reserved; only `chars` is implemented right now.
pub struct Tokenizer {
    vocab: HashMap<String, i64>,
    unknown_id: i64,
    bos_id: Option<i64>,
    eos_id: Option<i64>,
    mode: TokenizerMode,
}

#[derive(Debug, Clone, Copy)]
enum TokenizerMode {
    Chars,
}

#[derive(Debug, Deserialize)]
struct RawTokenizer {
    vocab: HashMap<String, i64>,
    #[serde(default)]
    unknown_id: Option<i64>,
    #[serde(default)]
    bos_id: Option<i64>,
    #[serde(default)]
    eos_id: Option<i64>,
    #[serde(default)]
    mode: Option<String>,
}

impl Tokenizer {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("tokenizer.json を読めません ({}): {error}", path.display()))?;
        let parsed: RawTokenizer = serde_json::from_str(&raw)
            .map_err(|error| format!("tokenizer.json のJSONが不正です: {error}"))?;
        if parsed.vocab.is_empty() {
            return Err("tokenizer.json の vocab が空です。".to_string());
        }
        let mode = match parsed.mode.as_deref() {
            Some("chars") | None => TokenizerMode::Chars,
            Some(other) => {
                return Err(format!(
                    "tokenizer.json の mode `{other}` は未対応です。対応値: chars"
                ))
            }
        };
        let unknown_id = parsed.unknown_id.unwrap_or(0);
        Ok(Self {
            vocab: parsed.vocab,
            unknown_id,
            bos_id: parsed.bos_id,
            eos_id: parsed.eos_id,
            mode,
        })
    }

    pub fn encode(&self, text: &str) -> Result<Vec<i64>, SynthError> {
        if text.trim().is_empty() {
            return Err(SynthError::EmptyText);
        }
        let mut ids: Vec<i64> = Vec::with_capacity(text.chars().count() + 2);
        if let Some(bos) = self.bos_id {
            ids.push(bos);
        }
        match self.mode {
            TokenizerMode::Chars => {
                for ch in text.chars() {
                    let key: String = ch.to_string();
                    let id = self.vocab.get(&key).copied().unwrap_or(self.unknown_id);
                    ids.push(id);
                }
            }
        }
        if let Some(eos) = self.eos_id {
            ids.push(eos);
        }
        Ok(ids)
    }
}
