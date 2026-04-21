export type PromptOptions = {
  sourceType: string;
  audience: string;
  style: string;
  length: string;
  purpose: string;
};

export const sourceTypeInstructions: Record<string, string> = {
  "PowerPoint / スライド資料": "スライド番号や箇条書きをそのまま読み上げず、話の流れが分かるナレーションに再構成してください。",
  "Excel / 表データ": "表の列や数値を、重要な傾向、比較、例外が音声で理解できる説明に変換してください。",
  "議事録 / 会議メモ": "決定事項、論点、未決事項、担当者を整理し、時系列より理解しやすさを優先してください。",
  "メール / チャットログ": "発言者名の羅列を避け、背景、依頼、結論、次のアクションが分かる原稿にしてください。",
  "Webページ / 記事": "見出し、リンク、脚注を自然な説明に置き換え、出典にない推測は追加しないでください。",
  "マニュアル / 手順書": "手順の目的、注意点、失敗しやすい点を音声で追いやすい順序にしてください。",
  その他: "元資料の構造に合わせて、音声だけで理解できる章立てに再構成してください。"
};

export function buildPrompt(options: PromptOptions): string {
  const extra = sourceTypeInstructions[options.sourceType] ?? sourceTypeInstructions["その他"];
  return `あなたは、元資料をもとに音声で聞きやすいオーディオブック用ナレーション原稿を作る編集者です。

以下の条件で、元資料だけを根拠に Markdown 原稿を作成してください。

- 元資料種別: ${options.sourceType}
- 対象読者: ${options.audience}
- 文体: ${options.style}
- 長さ: ${options.length}
- 目的: ${options.purpose}

追加指示:
${extra}

出力ルール:
- 音声だけで理解できる自然な説明にする。
- 表、図、スライド、箇条書き、URL、脚注はナレーションとして分かる表現に変換する。
- 出典にない内容、推測、創作は追加しない。
- Markdown の front matter を先頭に付ける。
- 章は # 見出しで区切る。
- 適切な位置に [pause:short]、[pause:medium]、[pause:long] を入れる。
- 最後に # 原稿作成メモ を作り、省略した内容と確認が必要な点を書く。

front matter 例:
---
title: 生成されたタイトル
source_type: ${options.sourceType}
audience: ${options.audience}
language: ja-JP
style: ${options.style}
version: 1
---`;
}
