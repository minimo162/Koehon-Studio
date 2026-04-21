# design.md

# AI原稿対応オーディオブック作成アプリ 設計書

## 1. 設計方針

本アプリは、任意の元資料から外部AIが生成した Markdown 形式のオーディオブック用原稿を読み込み、ローカルTTSエンジンで音声化する Windows 向けデスクトップアプリである。

設計上の重要な方針は以下のとおり。

1. 元資料解析と音声生成を分離する。
2. 外部AIで作成された「音声用原稿」を入力の中心にする。
3. 原稿は Markdown を標準とする。
4. TTS処理はアプリ本体から分離し、sidecar として実行する。
5. 長文処理は章・段落・チャンク単位で管理する。
6. 失敗チャンクだけを再生成できるようにする。
7. 将来、他のTTSエンジンや直接ファイル解析に拡張できる構成にする。

## 2. 全体アーキテクチャ

```text
┌──────────────────────────────────────────────┐
│ Tauri Desktop App                            │
│                                              │
│  ┌────────────────────────────────────────┐  │
│  │ Svelte Frontend                         │  │
│  │ - 原稿読み込みUI                         │  │
│  │ - 原稿プレビュー                         │  │
│  │ - 章一覧                                 │  │
│  │ - 生成キュー表示                         │  │
│  │ - 音声プレビュー                         │  │
│  │ - プロンプトコピー画面                   │  │
│  └────────────────────────────────────────┘  │
│                      │                       │
│                      ▼                       │
│  ┌────────────────────────────────────────┐  │
│  │ Tauri Rust Core                         │  │
│  │ - ファイル読み書き                       │  │
│  │ - appDataDir 管理                       │  │
│  │ - sidecar 起動・停止                     │  │
│  │ - 音声結合                               │  │
│  │ - export 処理                            │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
                       │ HTTP localhost
                       ▼
┌──────────────────────────────────────────────┐
│ TTS Backend Sidecar                           │
│ - FastAPI または軽量HTTPサーバー              │
│ - MOSS-TTS-Nano ONNX 推論                     │
│ - テキストチャンク → WAV生成                  │
│ - health check                                │
└──────────────────────────────────────────────┘
```

## 3. 技術スタック

### 3.1 フロントエンド

- Svelte
- TypeScript
- Vite
- Tauri JavaScript API
- Tauri shell plugin
- Tauri dialog / fs / path 系API

### 3.2 デスクトップ基盤

- Tauri v2 系
- Rust
- Windows x64 を主対象

### 3.3 TTSバックエンド

- Python sidecar
- FastAPI
- Uvicorn
- ONNX Runtime
- MOSS-TTS-Nano ONNX
- 音声処理用ライブラリ
  - MVP: WAV 処理中心
  - 将来: ffmpeg 連携で MP3 / M4B

### 3.4 データ形式

- 原稿: Markdown / txt
- プロジェクト: JSON + Markdown + 音声ファイル
- 音声: WAV を基本とする
- 設定: JSON

## 4. コンポーネント構成

```text
src/
  App.svelte
  lib/
    api/
      ttsClient.ts
      tauriCommands.ts
    manuscript/
      parser.ts
      chunker.ts
      tags.ts
      normalizer.ts
    project/
      projectStore.ts
      projectTypes.ts
    prompt/
      promptTemplates.ts
      promptBuilder.ts
    audio/
      audioPlayer.ts
      exportTypes.ts
    stores/
      appSettings.ts
      generationQueue.ts
      manuscriptStore.ts
  routes or views/
    HomeView.svelte
    ManuscriptView.svelte
    GenerationView.svelte
    PromptView.svelte
    SettingsView.svelte
    LogsView.svelte

src-tauri/
  src/
    main.rs
    commands/
      file_commands.rs
      sidecar_commands.rs
      audio_commands.rs
      project_commands.rs
    services/
      sidecar_manager.rs
      project_service.rs
      audio_merge_service.rs
  binaries/
    tts-backend-x86_64-pc-windows-msvc.exe

backend/
  tts_server.py
  tts_engine/
    moss_onnx_engine.py
    base.py
  audio/
    wav_utils.py
  config.py
```

## 5. 画面設計

### 5.1 ホーム画面

目的: 作業開始地点。

表示要素:

- 新規プロジェクト作成
- 既存プロジェクトを開く
- Markdown原稿を読み込む
- AI原稿生成プロンプトをコピー
- 最近使ったプロジェクト

### 5.2 原稿画面

目的: 原稿確認・編集・章分割確認。

表示要素:

- 原稿エディタ
- Markdownプレビュー
- 章一覧
- メタ情報表示
- 読み上げ対象 ON/OFF
- 原稿保存
- チャンク分割プレビュー

### 5.3 生成画面

目的: TTS生成の管理。

表示要素:

- 全体生成ボタン
- 選択章生成ボタン
- 生成停止ボタン
- 章ごとの進捗
- チャンクごとの状態
- エラー表示
- 再生成ボタン

### 5.4 音声プレビュー画面

目的: 生成済み音声の確認。

表示要素:

- 章単位の再生
- チャンク単位の再生
- 生成済みファイル一覧
- 再生成導線
- 書き出しボタン

### 5.5 プロンプト画面

目的: 外部AI用の原稿生成プロンプトを作成・コピー。

表示要素:

- 元資料種別選択
- 対象読者選択
- 文体選択
- 長さ選択
- 目的選択
- プロンプトプレビュー
- コピー
- テンプレート保存

### 5.6 設定画面

目的: TTS、出力、保存先、モデル設定。

表示要素:

- TTSエンジン選択
- モデルディレクトリ
- 出力ディレクトリ
- CPUスレッド数
- 既定話者
- チャンク最大文字数
- pause秒数
- 出力形式
- ffmpegパス

### 5.7 ログ画面

目的: 問題調査。

表示要素:

- sidecar起動ログ
- TTS生成ログ
- エラーログ
- ログファイルを開く

## 6. データモデル

### 6.1 Project

```ts
export type Project = {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  projectDir: string;
  manuscriptPath: string;
  metadata: ManuscriptMetadata;
  chapters: Chapter[];
  settings: ProjectSettings;
  generation: GenerationState;
};
```

### 6.2 ManuscriptMetadata

```ts
export type ManuscriptMetadata = {
  title?: string;
  source_type?: string;
  source?: string;
  audience?: string;
  language?: string;
  style?: string;
  version?: number | string;
};
```

### 6.3 Chapter

```ts
export type Chapter = {
  id: string;
  title: string;
  level: number;
  order: number;
  rawMarkdown: string;
  plainText: string;
  includeInNarration: boolean;
  chunks: ManuscriptChunk[];
  audio?: ChapterAudioState;
};
```

### 6.4 ManuscriptChunk

```ts
export type ManuscriptChunk = {
  id: string;
  chapterId: string;
  order: number;
  type: "text" | "pause";
  text?: string;
  pauseMs?: number;
  tags: ManuscriptTag[];
  status: "pending" | "generating" | "done" | "failed" | "skipped";
  audioPath?: string;
  error?: string;
  retryCount: number;
};
```

### 6.5 ManuscriptTag

```ts
export type ManuscriptTag = {
  raw: string;
  name: "pause" | "voice" | "speed" | "chapter" | "unknown";
  value?: string;
};
```

### 6.6 GenerationState

```ts
export type GenerationState = {
  status: "idle" | "running" | "paused" | "stopping" | "completed" | "failed";
  currentChapterId?: string;
  currentChunkId?: string;
  totalChunks: number;
  completedChunks: number;
  failedChunks: number;
  startedAt?: string;
  finishedAt?: string;
};
```

### 6.7 ProjectSettings

```ts
export type ProjectSettings = {
  ttsEngine: "moss-tts-nano-onnx";
  voice?: string;
  cpuThreads: number;
  maxChunkChars: number;
  pauseShortMs: number;
  pauseMediumMs: number;
  pauseLongMs: number;
  outputSampleRate?: number;
  exportFormat: "wav" | "mp3" | "m4b";
  includeManuscriptMemo: boolean;
};
```

## 7. プロジェクトファイル構成

```text
projects/
  {projectId}/
    project.json
    manuscript.md
    chunks/
      chapter-001.json
      chapter-002.json
    audio/
      chunks/
        chapter-001_chunk-001.wav
        chapter-001_chunk-002.wav
      chapters/
        chapter-001.wav
        chapter-002.wav
      exports/
        audiobook.wav
    logs/
      generation.log
```

### 7.1 project.json 例

```json
{
  "id": "proj_20260421_001",
  "title": "研修資料のオーディオブック",
  "createdAt": "2026-04-21T10:00:00+09:00",
  "updatedAt": "2026-04-21T10:30:00+09:00",
  "manuscriptPath": "manuscript.md",
  "metadata": {
    "title": "研修資料のオーディオブック",
    "source_type": "PowerPoint",
    "source": "2026年度 研修資料",
    "language": "ja-JP"
  },
  "settings": {
    "ttsEngine": "moss-tts-nano-onnx",
    "cpuThreads": 4,
    "maxChunkChars": 450,
    "pauseShortMs": 500,
    "pauseMediumMs": 1000,
    "pauseLongMs": 2000,
    "exportFormat": "wav",
    "includeManuscriptMemo": false
  }
}
```

## 8. 原稿パーサー設計

### 8.1 処理フロー

```text
raw markdown
  ↓
front matter 抽出
  ↓
特殊タグ検出
  ↓
章分割
  ↓
読み上げ対象判定
  ↓
Markdown記法の簡易除去
  ↓
テキスト正規化
  ↓
チャンク分割
```

### 8.2 front matter 抽出

- `---` で囲まれた先頭ブロックを YAML front matter とみなす。
- MVPでは既知キーのみ抽出する。
- 不明キーは保持してもよい。
- YAML解析に失敗した場合、front matter を本文として扱わず、警告を表示する。

### 8.3 章分割

- `# ` で始まる行を章境界とする。
- `## ` 以降は章内セクションとして扱う。
- `# 原稿作成メモ` は初期状態で読み上げ対象外にする。
- 見出しがない場合は全文を `本文` 章として扱う。

### 8.4 特殊タグ処理

特殊タグは本文中の以下の形式を対象にする。

```text
[pause:short]
[pause:medium]
[pause:long]
[voice:narrator]
[speed:normal]
[chapter:end]
```

処理方針:

- `[pause:*]` は pause チャンクに変換する。
- `[voice:*]` は後続チャンクのタグとして保持する。
- `[speed:*]` は後続チャンクのタグとして保持する。
- 未知タグは警告表示し、本文からは除去しないか、設定に応じて除去する。

## 9. チャンク分割設計

### 9.1 目的

長文をTTSに直接渡すと不安定になるため、自然な文境界で分割する。

### 9.2 基本ルール

- 最大文字数の既定値は 450 文字。
- 推奨範囲は 200〜600 文字。
- 段落区切りを優先する。
- 次に句点、疑問符、感嘆符を優先する。
- それでも長い場合は読点や空白で分割する。
- 最後の手段として最大文字数で強制分割する。

### 9.3 分割優先順位

```text
1. [pause:*]
2. 空行
3. 句点「。」
4. 疑問符「？」、感嘆符「！」
5. 読点「、」
6. 最大文字数
```

### 9.4 チャンク化例

入力:

```markdown
# 第1章 背景

この章では、背景を説明します。
[pause:short]
まず、重要なのは目的を明確にすることです。
```

出力:

```ts
[
  { type: "text", text: "この章では、背景を説明します。" },
  { type: "pause", pauseMs: 500 },
  { type: "text", text: "まず、重要なのは目的を明確にすることです。" }
]
```

## 10. TTS Backend API設計

### 10.1 起動方式

Tauri shell plugin で sidecar を起動する。

```text
tts-backend-x86_64-pc-windows-msvc.exe
  --host 127.0.0.1
  --port 18083
  --model-dir {appData}/models
  --output-dir {project}/audio/chunks
  --cpu-threads 4
```

### 10.2 Health API

```http
GET /health
```

レスポンス例:

```json
{
  "ok": true,
  "engine": "moss-tts-nano-onnx",
  "voices": [
    { "id": "default", "name": "default" }
  ]
}
```

### 10.3 Synthesize API

```http
POST /synthesize
```

リクエスト例:

```json
{
  "request_id": "chunk_001",
  "text": "この章では、背景を説明します。",
  "voice": "default",
  "seed": 1234,
  "output_path": "C:/Users/.../audio/chunks/chapter-001_chunk-001.wav"
}
```

レスポンス例:

```json
{
  "ok": true,
  "request_id": "chunk_001",
  "audio_path": "C:/Users/.../audio/chunks/chapter-001_chunk-001.wav",
  "sample_rate": 48000,
  "elapsed_seconds": 3.25
}
```

### 10.4 Cancel API

MVPでは生成中の1リクエストを強制中断できなくてもよい。キュー停止はフロントエンド側で行い、現在の生成完了後に停止する。

将来的には以下を追加する。

```http
POST /cancel
```

## 11. TTSクライアント設計

```ts
export interface TtsClient {
  health(): Promise<TtsHealth>;
  synthesize(req: SynthesizeRequest): Promise<SynthesizeResult>;
}
```

```ts
export type SynthesizeRequest = {
  requestId: string;
  text: string;
  voice?: string;
  seed?: number;
  outputPath: string;
};

export type SynthesizeResult = {
  ok: boolean;
  requestId: string;
  audioPath: string;
  sampleRate: number;
  elapsedSeconds: number;
};
```

## 12. 生成キュー設計

### 12.1 状態遷移

```text
pending → generating → done
                    ↘ failed
failed → pending → generating → done
pending → skipped
```

### 12.2 キュー処理

1. 生成対象チャンクを取得する。
2. pause チャンクは音声生成せず、後段の結合処理で無音として扱う。
3. text チャンクを順番に `/synthesize` へ送る。
4. 成功したら `audioPath` を保存する。
5. 失敗したら `failed` にする。
6. 停止要求があれば、現在のチャンク完了後に停止する。

### 12.3 並列化

MVPでは逐次生成とする。

理由:

- TTSエンジンのメモリ消費を抑える。
- 音声の話者・seed・順序管理を単純化する。
- Windows上の安定性を優先する。

将来的には、エンジンが対応する場合のみ並列数を設定可能にする。

## 13. 音声結合設計

### 13.1 MVP

- WAVのみを対象とする。
- すべて同一サンプルレート・チャンネル数であることを前提に結合する。
- pause チャンクでは無音WAVデータを挿入する。

### 13.2 結合単位

- チャンク → 章WAV
- 章WAV → 全体WAV

### 13.3 将来拡張

- ffmpeg による MP3 変換。
- ffmpeg による M4B 変換。
- 章メタデータ埋め込み。
- ラウドネスノーマライズ。
- 先頭・末尾無音の調整。

## 14. プロンプト生成設計

### 14.1 入力パラメータ

```ts
export type PromptOptions = {
  sourceType: "auto" | "pdf" | "word" | "powerpoint" | "excel" | "web" | "meeting" | "email" | "chat" | "manual" | "memo" | "other";
  audience: "general" | "business" | "beginner" | "expert" | "training";
  length: "short" | "standard" | "detailed";
  style: "calm" | "training" | "news" | "friendly";
  purpose: "learning" | "sharing" | "summary" | "training" | "review";
  requireSourceOnly: boolean;
  includeManuscriptMemo: boolean;
};
```

### 14.2 出力

Markdown原稿を作るためのプロンプト文字列。

### 14.3 資料タイプ別追加指示

- PowerPoint: スライドの箇条書きを自然な講義文に変換する。
- Excel: すべての数値ではなく、傾向・比較・判断材料を説明する。
- 議事録: 背景、論点、決定事項、未決事項、次のアクションに再構成する。
- メール/チャット: 時系列を整理し、背景、依頼、回答、注意点をまとめる。
- Webページ: ナビゲーション、広告、不要URLを除外する。
- マニュアル: 目的、前提、手順、注意点、確認方法を説明する。

## 15. sidecar管理設計

### 15.1 起動時

1. appDataDir を取得する。
2. モデルディレクトリを確認する。
3. プロジェクト出力ディレクトリを作成する。
4. sidecar を起動する。
5. `/health` をポーリングする。
6. 成功したらアプリ状態を `backendReady` にする。

### 15.2 終了時

1. 生成中キューを停止する。
2. sidecar プロセスを終了する。
3. ログを保存する。

### 15.3 異常時

- health check に失敗したら再起動ボタンを表示する。
- 連続再起動は回数制限を設ける。
- stderr をログへ保存する。

## 16. セキュリティ設計

### 16.1 ローカルAPI制限

- backend は `127.0.0.1` のみに bind する。
- 将来的には起動時にランダムトークンを生成し、全APIに Authorization を要求する。

### 16.2 ファイルパス制限

- 出力先はプロジェクトディレクトリ配下を既定とする。
- 任意パス出力時はユーザーに明示的に保存先を選ばせる。
- 原稿タグからファイルパスやコマンドを実行しない。

### 16.3 原稿タグ制限

- 解釈対象タグはホワイトリスト化する。
- 未知タグは実行せず、警告または本文として扱う。

## 17. エラー設計

### 17.1 エラー分類

| 分類 | 例 | 表示 |
|---|---|---|
| FileError | 読み込み失敗、保存失敗 | ファイル名と原因 |
| ParseError | front matter不正、タグ不正 | 行番号と原因 |
| BackendError | sidecar起動失敗、health失敗 | 再起動ボタン |
| TtsError | 合成失敗 | チャンク番号と本文冒頭 |
| ExportError | 結合失敗、書き出し失敗 | 出力先と原因 |
| ConfigError | 設定値不正 | 項目名と許容範囲 |

### 17.2 ログ項目

- 時刻
- プロジェクトID
- 操作種別
- 対象章ID
- 対象チャンクID
- エラー種別
- メッセージ
- stack trace または backend stderr

## 18. テスト設計

### 18.1 ユニットテスト

- front matter 抽出
- Markdown章分割
- 特殊タグ解析
- チャンク分割
- プロンプト生成
- 設定バリデーション

### 18.2 結合テスト

- 原稿読み込みから章一覧表示まで
- チャンク生成からキュー登録まで
- TTS backend health check
- 1チャンク音声生成
- pause挿入ありのWAV結合

### 18.3 E2Eテスト

- Markdown原稿を読み込む。
- 章を選択する。
- 音声生成する。
- 章音声を再生する。
- 全体WAVを書き出す。
- プロジェクトを保存して再度開く。

### 18.4 手動確認

- Windows 10 / 11 で起動確認。
- sidecar同梱後の起動確認。
- モデル未配置時の挙動確認。
- 長文原稿の生成安定性確認。
- 日本語テキストの読み上げ品質確認。

## 19. MVP実装順序

1. Tauri + Svelte プロジェクト作成。
2. 原稿ファイル読み込み。
3. Markdown parser / chapter parser 実装。
4. 原稿プレビュー画面実装。
5. チャンク分割実装。
6. Python TTS backend の `/health` と `/synthesize` 実装。
7. sidecar 起動処理実装。
8. 生成キュー実装。
9. WAV保存・プレビュー実装。
10. WAV結合実装。
11. プロジェクト保存実装。
12. プロンプト生成画面実装。
13. 設定画面実装。
14. Windows向けビルド。

## 20. 将来の拡張設計

### 20.1 TTSエンジン抽象化

```ts
export interface TtsEngineAdapter {
  id: string;
  name: string;
  health(): Promise<TtsHealth>;
  synthesize(req: SynthesizeRequest): Promise<SynthesizeResult>;
  listVoices(): Promise<TtsVoice[]>;
}
```

これにより、将来的に以下へ拡張可能にする。

- MOSS-TTS-Nano ONNX
- 別のローカルTTS
- 商用クラウドTTS
- Windows標準音声

### 20.2 原稿入力抽象化

```ts
export interface ManuscriptImporter {
  id: string;
  extensions: string[];
  import(path: string): Promise<ImportedManuscript>;
}
```

将来的に以下の importer を追加する。

- MarkdownImporter
- TxtImporter
- EpubImporter
- DocxImporter
- PdfImporter
- HtmlImporter

### 20.3 音声出力抽象化

```ts
export interface AudioExporter {
  id: string;
  extension: string;
  export(req: ExportRequest): Promise<ExportResult>;
}
```

将来的に以下を追加する。

- WavExporter
- Mp3Exporter
- M4bExporter
