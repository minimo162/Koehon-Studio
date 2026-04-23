# Koehon Studio

AIが作成した Markdown / txt 原稿を読み込み、章分割、読み上げ用チャンク分割、ローカルTTS sidecar への生成依頼を行うデスクトップアプリです。

## 開発起動

```bash
pnpm install
pnpm dev                 # UI だけ (sidecar なし)
pnpm run tauri:dev       # Tauri アプリとして起動 (sidecar を自動ビルド&起動)
```

## 確認コマンド

```bash
pnpm test                # vitest
pnpm check               # svelte-check
pnpm build               # sidecar release + UI を dist/ へ
pnpm run build:check         # Tauri debug build (インストーラ生成なし)
pnpm run build:check:release # Tauri release build (インストーラ生成なし)
pnpm build:sidecar       # sidecar のみ (debug)
pnpm build:sidecar:release
```

`pnpm run build:check` / `pnpm run build:check:release` は `tauri build --no-bundle`
を使うため、NSIS/MSI の圧縮処理を待たずにアプリ本体のビルド確認だけできます。
生成物は debug が `src-tauri/target/debug/`、release が
`src-tauri/target/release/` に出ます。

## Windowsインストーラの配布形式

- **NSIS (`.exe`)**: 起動時に「現在のユーザーだけ / この PC 全体」を選択可能。ユーザーだけを選べば管理者権限なしで `%LOCALAPPDATA%\Programs\Koehon Studio\` にインストールされる (`tauri.conf.json` の `bundle.windows.nsis.installMode = "both"`)。
- **MSI (`.msi`)**: WiX が生成する per-machine インストーラ。Group Policy / Intune など企業配布向け。管理者権限が必要。

個人で別PCに入れるだけなら NSIS の方を使ってください。

## Windowsインストーラのビルド

### ローカル (Windows 実機)

1. Node.js 20+, pnpm, Rust (rustup), MSVC Build Tools, WebView2 Runtime を入れる
2. `pnpm install && pnpm tauri build`
3. `src-tauri/target/release/bundle/msi/*.msi` と `.../nsis/*.exe` が生成される

### GitHub Actions

`.github/workflows/windows-build.yml` を用意しています。
`Actions` タブから `Windows Build` を手動実行するか、`v*` タグを push すると `windows-latest` ランナーでインストーラが生成され、アーティファクトとしてダウンロードできます。

## ネイティブ sidecar のビルドと同梱

sidecar (`koehon-tts-sidecar`) は `scripts/build-sidecar.mjs` 経由でビルドし、
Tauri が期待する target triple 付きファイル名で `native-tts/sidecars/` に配置されます。

```text
native-tts/sidecars/koehon-tts-sidecar-x86_64-pc-windows-msvc.exe
native-tts/sidecars/koehon-tts-sidecar-x86_64-unknown-linux-gnu
native-tts/sidecars/koehon-tts-sidecar-aarch64-apple-darwin
```

`tauri.conf.json` の `bundle.externalBin` がこのパスを参照しているため、
`pnpm tauri build` で自動的にインストーラへ含まれます。

### 別 target を明示したい場合

```bash
node scripts/build-sidecar.mjs --release --target x86_64-pc-windows-msvc
```

## ONNX Runtime と MOSS-TTS-Nano モデル

### 自動ダウンロード

設定画面の「モデルダウンロード」パネルから、Hugging Face の公開リポジトリを 1クリックで取得できます:

| リポジトリ | サイズ | 用途 |
|---|---:|---|
| [OpenMOSS-Team/MOSS-TTS-Nano-100M-ONNX](https://huggingface.co/OpenMOSS-Team/MOSS-TTS-Nano-100M-ONNX) | 約672MB | TTS本体 (prefill / decode_step / local_decoder / local_cached_step / local_fixed_sampled_frame の 5-stage ONNX + external data + tokenizer.model) |
| [OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX](https://huggingface.co/OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX) | 約45MB | 波形 ↔ 音声トークン変換 ONNX |

ダウンロード先は `設定画面 → モデルディレクトリ` 配下に `moss-tts-nano/` / `moss-audio-tokenizer/` のサブフォルダとして作成されます。
TTS本体を落とした時点で `modelDirectory` が自動的に `moss-tts-nano/` を指すように更新されます。

### MOSS-TTS-Nano-100M-ONNX の構成

```text
<model-dir>/moss-tts-nano/
├── moss_tts_prefill.onnx                  Global transformer prefill graph
├── moss_tts_decode_step.onnx              Global transformer decode-step (KV cache)
├── moss_tts_local_decoder.onnx            Local decoder graph
├── moss_tts_local_cached_step.onnx        Local cached-step graph
├── moss_tts_local_fixed_sampled_frame.onnx Local frame sampling graph
├── moss_tts_global_shared.data            External weights (global)
├── moss_tts_local_shared.data             External weights (local)
├── tokenizer.model                        SentencePiece tokenizer
├── tts_browser_onnx_meta.json             ONNX runtime metadata
└── browser_poc_manifest.json              Browser integration manifest
```

実行時は autoregressive な Audio Tokenizer + LLM パイプライン:
text → (SentencePiece) → (global prefill) → (global decode_step loop with KV cache) → audio tokens → (audio-tokenizer decode) → 48kHz 2ch PCM。

現行の実装状況:

- **単一ファイル向けの汎用エンジン** (`engine/moss_onnx.rs`) — `model.onnx` + `tokenizer.json` + `config.json` を前提とするシンプル構成
- **MOSS-TTS-Nano 向けの 5段エンジン** (`engine/moss_tts_nano.rs`) — `tts_browser_onnx_meta.json` を検出し、5 つの ONNX Session + external data + 18 voices を読み込む scaffold。**autoregressive 生成ループ本体は未実装**

残りの実装は `docs/MOSS_PIPELINE.md` に (inspect 済みの ONNX I/O shapes を含む) 完全な仕様を記載しています。実装順序 / KV cache plumbing / sampling / codec decode までの設計を参照してください。

### 配置方針

| 成果物 | 配置先 | 備考 |
|---|---|---|
| ONNX Runtime DLL (`onnxruntime.dll` 等) | インストーラに同梱 (`bundle.resources`)、Tauri app 実行ファイルと同一ディレクトリ | Windows ビルドは CI が `onnxruntime-win-x64-1.20.1` を自動DL |
| MOSS-TTS-Nano モデルファイル | ユーザー環境の任意ディレクトリ (推奨: `%APPDATA%/Koehon Studio/models/moss-tts-nano/`) | 設定画面 → モデルディレクトリで指定 |
| プロジェクト / 生成音声 / ログ | `%APPDATA%/Koehon Studio/` 配下 | |

### モデルディレクトリの期待構成

sidecar は `--model-dir` で渡されたディレクトリから以下のファイルを探します:

```text
<model-dir>/
├── model.onnx         MOSS-TTS-Nano 互換の ONNX ファイル
├── tokenizer.json     { "vocab", "unknown_id", "bos_id", "eos_id", "mode" }
└── config.json        { "sample_rate", "channels", "voices", 入出力名 }
```

#### tokenizer.json

```json
{
  "vocab": { "a": 0, "い": 1, "う": 2 },
  "unknown_id": 0,
  "bos_id": 1,
  "eos_id": 2,
  "mode": "chars"
}
```

`mode: "chars"` は「1文字=1トークン」のパススルー実装。本番用の G2P / 音素化が必要な場合は、
利用する TTS モデルに合わせて呼び出し側で事前変換するか、将来 `mode: "phoneme"` を追加してください。

#### config.json (すべて省略可能)

```json
{
  "sample_rate": 24000,
  "channels": 1,
  "text_input_name": "input_ids",
  "speaker_input_name": "speaker_id",
  "seed_input_name": null,
  "audio_output_name": "audio",
  "voices": [
    { "id": "narrator", "name": "ナレーター", "speaker_id": 0 },
    { "id": "calm",     "name": "穏やか",     "speaker_id": 1 }
  ]
}
```

#### モデル I/O の期待

- 入力 `input_ids`: `int64 [1, seq]` — tokenizer の出力
- 入力 `speaker_id` (あれば): `int64 [1]` — 選択 voice の `speaker_id` を渡す
- 入力 `seed` (あれば): `int64 [1]` — 乱数シード
- 出力 `audio`: `float32 [1, samples]` または `[samples]` — `-1.0 ... 1.0` のモノラル PCM

### モデルが無いとき

sidecar は `/health` で `engine=koehon-test-tone` を返し、`diagnostics` に
`model.dir_unset` / `model.missing` / `tokenizer.missing` / `onnx.runtime_missing`
のいずれかを含めます。UI はこの診断をログ画面で表示し、生成はテストトーンを出力します。

## 実装済みの範囲

- Markdown / txt 原稿読み込み
- YAML front matter 抽出
- `#` 見出しによる章分割
- `# 原稿作成メモ` の初期除外
- `[pause:short]`、`[pause:medium]`、`[pause:long]` のチャンク化
- 文境界を優先したチャンク分割
- 原稿編集と localStorage 下書き保存
- プロジェクト保存・復元
- 生成キュー、進捗、失敗チャンク再生成
- TTS sidecar API クライアント (health / synthesize)
- sidecar の自動起動・停止 (Tauri shell plugin)
- WAV 結合 (章単位 / 全体) とエクスポート
- 外部AI用プロンプト生成とコピー
- 設定保存 (`settings.json`)
- コマンドパレット (⌘/Ctrl+K)、キーボードショートカット、ドラッグ&ドロップ読み込み
- Tauri v2 の Windows ビルド構成とCI

## 未実装の主要範囲

- MOSS-TTS-Nano ONNX 推論 (現状 sidecar はテストトーン)
- ONNX Runtime DLL / モデルの同梱資材
- MP3 / M4B エクスポート
- 読み替え辞書・ルビ・複数話者
