# MOSS-TTS-Nano 推論パイプライン実装仕様

このドキュメントは [`OpenMOSS-Team/MOSS-TTS-Nano-100M-ONNX`](https://huggingface.co/OpenMOSS-Team/MOSS-TTS-Nano-100M-ONNX) と音声コーデック [`OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX`](https://huggingface.co/OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX) を使って、Koehon-Studio の native-tts sidecar で実際の音声合成を動かすための完全な実装仕様です。

`tts_browser_onnx_meta.json` / `browser_poc_manifest.json` / 各 `.onnx` を実際に inspect して得た具体的な情報に基づいて書いています。

---

## 1. アーキテクチャ概要

MOSS-TTS-Nano は **hierarchical autoregressive** な LLM-based TTS。テキスト + 音声トークンを 1 フレーム = 17 チャネル (1 text + 16 音声コードブック) のストリームとして扱います。

```
text string
  │
  ▼
SentencePiece tokenize (tokenizer.model, vocab 16384)
  │
  ▼
Build input sequence [1, seq, 17]
  (user_prompt_prefix + [voice reference frames] + user_prompt_after_reference + tokenized text + assistant_prompt_prefix)
  │
  ▼
PREFILL (moss_tts_prefill.onnx)  ──►  global_hidden [1, seq, 768] + 12層 KV cache
  │
  ▼
FRAME LOOP (最大 max_new_frames=375 回)
  ├─ local_fixed_sampled_frame(global_hidden[:, -1], ...) ─► should_continue, frame_token_ids [1, 16]
  ├─ should_continue == 0 → break
  ├─ 新フレーム [text_pad, frame_token_ids...] (shape [1, 1, 17]) を組み立て
  └─ decode_step(新フレーム, past KV cache) ─► updated global_hidden + KV
  │
  ▼
収集した audio tokens [frames_count, 16]
  │
  ▼
CODEC DECODE (moss_audio_tokenizer_decode_full.onnx)
  │
  ▼
48kHz 2ch PCM waveform
```

---

## 2. 必要なファイル

### MOSS-TTS-Nano-100M-ONNX (約 672 MB)

```
<model-dir>/moss-tts-nano/
├── moss_tts_prefill.onnx               283 KB   グラフのみ (weights は external data)
├── moss_tts_decode_step.onnx           291 KB
├── moss_tts_local_decoder.onnx          49 KB
├── moss_tts_local_cached_step.onnx      54 KB
├── moss_tts_local_fixed_sampled_frame.onnx  471 KB
├── moss_tts_global_shared.data         420 MB   prefill/decode_step の weights
├── moss_tts_local_shared.data            XX MB  local_* の weights
├── tokenizer.model                     460 KB   SentencePiece binary
├── tts_browser_onnx_meta.json          (入出力名 + model_config)
└── browser_poc_manifest.json           (voices + prompt_templates + generation_defaults)
```

### MOSS-Audio-Tokenizer-Nano-ONNX (約 45 MB)

```
<model-dir>/moss-audio-tokenizer/
├── moss_audio_tokenizer_decode_full.onnx
├── ... (encode, external data)
└── codec_browser_onnx_meta.json
```

---

## 3. ONNX ファイルの入出力シェイプ

実ファイルを `python -c "import onnx; ..."` で inspect した結果。n_vq=16, hidden_size=768, global_layers=12, head_dim=64。

### 3.1 moss_tts_prefill.onnx

| 種別 | 名前 | dtype | shape |
|---|---|---|---|
| in | `input_ids` | INT32 | `[batch, prefill_seq, 17]` |
| in | `attention_mask` | INT32 | `[batch, prefill_seq]` |
| out | `global_hidden` | FLOAT | `[batch, prefill_seq, 768]` |
| out | `present_key_N` (N=0..11) | FLOAT | `[batch, prefill_seq, 12, 64]` |
| out | `present_value_N` (N=0..11) | FLOAT | `[batch, prefill_seq, 12, 64]` |

### 3.2 moss_tts_decode_step.onnx

| 種別 | 名前 | dtype | shape |
|---|---|---|---|
| in | `input_ids` | INT32 | `[batch, step_seq, 17]` |
| in | `past_valid_lengths` | INT32 | `[batch]` |
| in | `past_key_N` (N=0..11) | FLOAT | `[batch, past_seq, 12, 64]` |
| in | `past_value_N` (N=0..11) | FLOAT | `[batch, past_seq, 12, 64]` |
| out | `global_hidden` | FLOAT | `[batch, step_seq, 768]` |
| out | `present_key_N` (N=0..11) | FLOAT | `[batch, total_seq, 12, 64]` |
| out | `present_value_N` (N=0..11) | FLOAT | `[batch, total_seq, 12, 64]` |

### 3.3 moss_tts_local_fixed_sampled_frame.onnx (★ 使用する sample_mode)

| 種別 | 名前 | dtype | shape |
|---|---|---|---|
| in | `global_hidden` | FLOAT | `[batch, 768]` |
| in | `repetition_seen_mask` | INT32 | `[batch, 16, 1024]` |
| in | `assistant_random_u` | FLOAT | `[batch]` |
| in | `audio_random_u` | FLOAT | `[batch, 16]` |
| out | `should_continue` | INT32 | `[batch, 1]` |
| out | `frame_token_ids` | INT32 | `[batch, 16]` |

★ サンプリング (temperature / top_p / top_k / repetition_penalty) は ONNX グラフ内部に定数として埋め込まれている。`fixed_sampled_frame_constants` 参照。

### 3.4 moss_tts_local_cached_step.onnx (alternative sample_mode)

| 種別 | 名前 | dtype | shape |
|---|---|---|---|
| in | `global_hidden` | FLOAT | `[batch, 768]` |
| in | `text_token_id` | INT32 | `[batch]` |
| in | `audio_token_id` | INT32 | `[batch]` |
| in | `channel_index` | INT32 | `[batch]` |
| in | `step_type` | INT32 | `[batch]` |
| in | `past_valid_lengths` | INT32 | `[batch]` |
| in | `local_past_key_0` | FLOAT | `[batch, local_past_seq, 12, 64]` |
| in | `local_past_value_0` | FLOAT | `[batch, local_past_seq, 12, 64]` |
| out | `text_logits` | FLOAT | `[batch, 16384]` |
| out | `audio_logits` | FLOAT | `[batch, 16, 1024]` |
| out | `local_present_key_0` | FLOAT | `[batch, local_total_seq, 12, 64]` |
| out | `local_present_value_0` | FLOAT | `[batch, local_total_seq, 12, 64]` |

manifest の `generation_defaults.sample_mode = "fixed"` を採用するので、初期実装では 3.3 を使い、3.4 は使わない。

### 3.5 moss_tts_local_decoder.onnx (補助)

`[1, ...]` 決め打ち。初期実装では未使用。

---

## 4. 入力シーケンスの組み立て

入力 `input_ids` の shape は `[1, seq, 17]`。列ごとに 17 個のトークン = `[text_token, audio_code_0, ..., audio_code_15]`。

### 4.1 音声生成用のプロンプト構造

`browser_poc_manifest.json` の `prompt_templates` より:

```
[user_prompt_prefix_token_ids]        ← 固定12トークン
[voice prompt_audio_codes 各フレーム]  ← 選択voiceに応じて98〜180フレーム
[user_prompt_after_reference_token_ids] ← 固定56トークン
[合成したいテキストをSentencePieceでトークン化したもの]
[assistant_prompt_prefix_token_ids]   ← 固定6トークン (最後に audio_assistant_slot_token_id=9 が来る)
```

### 4.2 行ごとの 17 チャネルの埋め方

| 種別 | text チャネル [,0] | audio チャネル [,1..17] |
|---|---|---|
| prompt_prefix フレーム | 対応 token_id | 全て `audio_pad_token_id = 1024` |
| voice reference フレーム | `audio_user_slot_token_id = 8` | voice の prompt_audio_codes (shape=[N, 16]) |
| after_reference フレーム | 対応 token_id | 全て `1024` |
| テキスト本文フレーム | sp token_id | 全て `1024` |
| assistant_prefix フレーム | 対応 token_id | 全て `1024` |

→ prefill に渡す `input_ids` 完成。`attention_mask` は全部 1 で埋める (seq 長分)。

### 4.3 重要な特殊トークン

`model_config` より:

| 名前 | 値 |
|---|---:|
| pad_token_id | 3 |
| im_start_token_id | 4 |
| im_end_token_id | 5 |
| audio_start_token_id | 6 |
| audio_end_token_id | 7 |
| audio_user_slot_token_id | 8 |
| audio_assistant_slot_token_id | 9 |
| audio_pad_token_id | 1024 |

---

## 5. 生成ループ

```
# 初期化
outputs = prefill(input_ids, attention_mask)
global_hidden = outputs.global_hidden       # [1, seq, 768]
kv = [(outputs.present_key_L, outputs.present_value_L) for L in 0..11]
past_valid_lengths = [seq]

audio_frames = []

for step in 0..max_new_frames:  # max 375
    last_hidden = global_hidden[:, -1, :]        # [1, 768]

    # 乱数を用意する
    repetition_seen_mask = 直近フレーム履歴から構築 ([1, 16, 1024] int32, 見たコードは 1)
    assistant_random_u = sample(Uniform(0, 1), shape=[1])
    audio_random_u = sample(Uniform(0, 1), shape=[1, 16])

    # サンプリング
    s = local_fixed_sampled_frame(last_hidden, repetition_seen_mask,
                                  assistant_random_u, audio_random_u)
    if s.should_continue[0, 0] == 0:
        break

    audio_tokens = s.frame_token_ids[0]          # [16]
    audio_frames.append(audio_tokens)

    # 次ステップの input を構築
    # text チャネルは assistant slot (9) を入れる想定 (※browser ref 実装で要確認)
    next_text_token = audio_assistant_slot_token_id  # 9
    next_input = [[[next_text_token, *audio_tokens]]]  # [1, 1, 17] int32

    # decode_step で KV を伸ばす
    d = decode_step(
        input_ids=next_input,
        past_valid_lengths=past_valid_lengths,
        past_keys=[k for k,_ in kv],
        past_values=[v for _,v in kv],
    )
    global_hidden = d.global_hidden              # [1, 1, 768]
    kv = [(d.present_key_L, d.present_value_L) for L in 0..11]
    past_valid_lengths[0] += 1

# audio_frames: List[[16]]  (frames_count フレーム)
```

### 5.1 repetition_seen_mask の作り方

`audio_repetition_penalty = 1.2` を ONNX グラフに反映するためのマスク。直近数フレームで使われた音声コードに 1 を立てて、ペナルティを ONNX 内部で適用させる。簡易実装としては「直近 N フレーム (たとえば 64) で出現したコードに 1」で OK。詳細は MOSS ブラウザ PoC の JS リファレンス実装参照。

### 5.2 乱数シード

`synthesize` request の `seed` を用いて `SmallRng::seed_from_u64(seed)` で Uniform(0,1) の乱数を生成する。サンプリングの確定性を得るため必ず固定シードを使うこと。

---

## 6. Codec Decode

`audio_frames` ([frames_count, 16] int32) を `moss_audio_tokenizer_decode_full.onnx` に渡して PCM を得る。

ファイル取得後に I/O shape を同様に inspect して本書に追記する。`codec_browser_onnx_meta.json` に入出力名と frame rate が書かれているはず。サンプリングレート 48 kHz / 2ch。

---

## 7. 18 個の builtin voices

`browser_poc_manifest.json.builtin_voices[]`:

| voice 名 | display_name | group | prompt codes shape |
|---|---|---|---|
| Junhao | CN 欢迎关注模思智能 | Chinese Male | 98 × 16 |
| Zhiming | CN 京味胡同闲聊 | Chinese Male | 98 × 16 |
| Weiguo | CN 说书 | Chinese Male | 140 × 16 |
| Xiaoyu | CN 明星 | Chinese Female | 180 × 16 |
| Yuewen | CN 机车 | Chinese Female | 102 × 16 |
| ... | | | |

合計 18、中国語 + 英語の男女混在。各 voice は事前計算された audio codes を持っているのでコーデックなしで即座にプロンプトに組み込める。

カスタム voice を追加したい場合は、ユーザーの参照音声を audio tokenizer encode ONNX に通して得た codes を保存すればよい。

---

## 8. SentencePiece トークナイザ

`tokenizer.model` は Google SentencePiece の protobuf 形式バイナリ。Rust では以下のいずれか:

- **[sentencepiece](https://crates.io/crates/sentencepiece) クレート** — Google の C++ 実装を cmake ビルドして FFI。確実だが Windows ビルドで cmake が要る (MSVC 上では OK)。
- **[tokenizers](https://crates.io/crates/tokenizers)** — HuggingFace 製。BPE/WordPiece/Unigram には対応するが SentencePiece binary 形式は直接は読まない。`sentencepiece → tokenizer.json` 変換が必要。

推奨は `sentencepiece` クレート。

---

## 9. 実装ステップ (チェックリスト)

- [x] MOSS ファイルを自動ダウンロード (Settings → モデルダウンロード)
- [x] `MossTtsNanoEngine` スキャフォールド: meta/manifest/tokenizer.model のロード、5 つの ONNX Session のロード (external data パス解決込み)、18 voices の表示
- [ ] SentencePiece 統合 (sentencepiece crate 追加)
- [ ] Prompt 構築: user_prefix + voice reference + after_reference + text + assistant_prefix を [seq, 17] に展開
- [ ] Prefill → global_hidden + KV を取得
- [ ] Frame loop:
  - [ ] repetition_seen_mask の管理
  - [ ] 乱数生成 (seed 適用)
  - [ ] local_fixed_sampled_frame 呼び出し
  - [ ] should_continue 判定
  - [ ] decode_step で KV を伸ばす
  - [ ] max_new_frames 上限
- [ ] MOSS-Audio-Tokenizer-Nano-ONNX の統合 (別 Session、別 .data)
- [ ] Audio tokens → 48 kHz 2ch PCM デコード
- [ ] voice が未指定時の既定: 最初の voice (`builtin_voices[0]`) を使う
- [ ] WAV 書き出しで 2ch を正しく扱う (現状 WAV writer は mono 前提)
- [ ] エラー経路: 各 ONNX 実行失敗・テンソル shape 不一致・token 化失敗ごとに `SynthError` を返す

---

## 10. 参考実装

- [OpenMOSS/MOSS-TTS-Nano-Reader](https://github.com/OpenMOSS/MOSS-TTS-Nano-Reader) — onnxruntime-web を使ったブラウザ参照実装。Rust 移植の際の答え合わせに最も有用。
- [OpenMOSS/MOSS-TTS-Nano](https://github.com/OpenMOSS/MOSS-TTS-Nano) — PyTorch 元実装。サンプリング内部と prompt 組み立ての正解が読める。

---

## 11. 現在の Koehon-Studio における対応状況

| コンポーネント | 状態 | 場所 |
|---|---|---|
| ORT Session 抽象 | 実装済 | `native-tts/src/engine/mod.rs` |
| 単一 .onnx 汎用エンジン | 実装済 (動作未検証) | `native-tts/src/engine/moss_onnx.rs` |
| MOSS 5段エンジン scaffold | 実装済 (synthesize はスタブ) | `native-tts/src/engine/moss_tts_nano.rs` |
| Diagnostic system | 実装済 | `/health` response |
| モデル自動 DL | 実装済 | `src/lib/api/modelDownloader.ts` |
| SentencePiece | **未** | — |
| Prompt builder | **未** | — |
| Prefill/decode loop | **未** | — |
| Audio codec decode | **未** | — |
| 2ch WAV 出力 | **未** (現状 mono 固定) | `native-tts/src/main.rs::write_pcm16_wav` |

追加実装量の目安: 800〜1200 行の Rust。最も risky な箇所は KV cache の tensor slicing と repetition_seen_mask の正しさ。PyTorch/JS 参照実装と出力 tensor を一致させて回帰テストを書きながら進めること。
