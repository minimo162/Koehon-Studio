# Koehon Studio

AIが作成した Markdown / txt 原稿を読み込み、章分割、読み上げ用チャンク分割、ローカルTTS sidecar への生成依頼を行うデスクトップアプリです。

## 開発起動

```bash
pnpm install
pnpm dev                 # UI だけ (sidecar なし)
pnpm tauri dev           # Tauri アプリとして起動 (sidecar を自動ビルド&起動)
```

## 確認コマンド

```bash
pnpm test                # vitest
pnpm check               # svelte-check
pnpm build               # sidecar release + UI を dist/ へ
pnpm build:sidecar       # sidecar のみ (debug)
pnpm build:sidecar:release
```

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

## ONNX Runtime と MOSS-TTS-Nano モデルの配置方針

初期版では以下の方針を採用しています (tasks 26 / 27 の決定):

| 成果物 | 配置先 | 理由 |
|---|---|---|
| ONNX Runtime DLL (`onnxruntime.dll` など) | インストール先 `resources/runtime/` に sidecar と同梱予定 | バージョン固定を保証したいため |
| MOSS-TTS-Nano ONNX モデル | ユーザー環境の `%APPDATA%/Koehon Studio/models/moss-tts-nano/` | 数百MBあるためインストーラに入れずにユーザー配置または将来の初回ダウンロードに任せる |
| プロジェクト / 生成音声 / ログ | `%APPDATA%/Koehon Studio/` 配下 | |

sidecar は起動時に model ディレクトリと ONNX Runtime の両方を診断し、
欠落していれば `/health` レスポンスにエラー理由を含める想定です (tasks 13 の続き)。

配置が決まるまで、現在同梱されている sidecar は「テストトーンを生成するダミー実装」のため、
インストーラを配っても Python や ONNX Runtime は不要で、UIフロー全体を確認できます。

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
