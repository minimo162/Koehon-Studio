# Koehon Studio

AIが作成した Markdown / txt 原稿を読み込み、章分割、読み上げ用チャンク分割、ローカルTTS sidecar への生成依頼を行うデスクトップアプリです。

## 開発起動

```bash
pnpm install
pnpm dev
```

ネイティブ TTS sidecar は未実装です。画面の「生成」は sidecar API に接続するための UI とキュー処理までを実装しています。

## 確認コマンド

```bash
pnpm test
pnpm check
pnpm build
```

## 実装済みの範囲

- Markdown / txt 原稿読み込み
- YAML front matter 抽出
- `#` 見出しによる章分割
- `# 原稿作成メモ` の初期除外
- `[pause:short]`、`[pause:medium]`、`[pause:long]` のチャンク化
- 文境界を優先したチャンク分割
- 原稿編集と localStorage 下書き保存
- 生成キュー、進捗、失敗表示
- TTS sidecar API クライアント
- 外部AI用プロンプト生成とコピー
- 設定保存
- Tauri v2 の最小プロジェクト設定

## 未実装の主要範囲

- ネイティブ TTS sidecar
- MOSS-TTS-Nano ONNX 推論
- WAV 書き出しと結合
- プロジェクト保存・復元
