import { get, writable } from "svelte/store";
import { parseManuscript, type ParsedManuscript } from "../manuscript/parser";
import { defaultProjectSettings } from "../project/projectTypes";
import { appSettingsStore } from "./appSettings";

export type ManuscriptState = {
  raw: string;
  fileName?: string;
  filePath?: string;
  projectDir?: string;
  projectFilePath?: string;
  parsed?: ParsedManuscript;
  dirty: boolean;
  error?: string;
  chapterInclusion: Record<string, boolean>;
};

const initialRaw = `---
title: サンプル原稿
source_type: Markdown
audience: 学習者
language: ja-JP
style: 落ち着いたナレーション
version: 1
---

# はじめに

このオーディオブックでは、元資料の重要なポイントを音声だけで理解できるように整理します。
[pause:medium]

# 第1章 背景

まず、背景を確認します。資料の目的、対象者、前提条件を順番に説明します。

# 原稿作成メモ

## 確認が必要な点

この章は初期設定では読み上げ対象から除外されます。
`;

function parse(raw: string): ParsedManuscript {
  return parseManuscript(raw, get(appSettingsStore) ?? defaultProjectSettings);
}

export const manuscriptStore = writable<ManuscriptState>({
  raw: initialRaw,
  parsed: parse(initialRaw),
  dirty: false,
  chapterInclusion: {}
});

export function setManuscript(raw: string, fileName?: string, filePath?: string): void {
  try {
    manuscriptStore.set({ raw, fileName, filePath, parsed: parse(raw), dirty: false, chapterInclusion: {} });
  } catch (error) {
    manuscriptStore.set({ raw, fileName, filePath, dirty: false, error: String(error), chapterInclusion: {} });
  }
}

export function updateManuscript(raw: string): void {
  try {
    manuscriptStore.update((state) => ({ ...state, raw, parsed: parse(raw), dirty: true, error: undefined }));
  } catch (error) {
    manuscriptStore.update((state) => ({ ...state, raw, dirty: true, error: String(error) }));
  }
}

export function markSaved(): void {
  manuscriptStore.update((state) => ({ ...state, dirty: false }));
}

export function markSavedAs(fileName: string, filePath?: string): void {
  manuscriptStore.update((state) => ({ ...state, fileName, filePath, dirty: false }));
}

export function markProjectSaved(projectDir: string, projectFilePath: string, fileName = "manuscript.md", filePath?: string): void {
  manuscriptStore.update((state) => ({ ...state, projectDir, projectFilePath, fileName, filePath, dirty: false }));
}

export function restoreManuscriptProject(raw: string, options: {
  fileName?: string;
  filePath?: string;
  projectDir?: string;
  projectFilePath?: string;
  chapterInclusion?: Record<string, boolean>;
} = {}): void {
  try {
    manuscriptStore.set({
      raw,
      fileName: options.fileName,
      filePath: options.filePath,
      projectDir: options.projectDir,
      projectFilePath: options.projectFilePath,
      parsed: parse(raw),
      dirty: false,
      chapterInclusion: options.chapterInclusion ?? {}
    });
  } catch (error) {
    manuscriptStore.set({
      raw,
      fileName: options.fileName,
      filePath: options.filePath,
      projectDir: options.projectDir,
      projectFilePath: options.projectFilePath,
      dirty: false,
      error: String(error),
      chapterInclusion: options.chapterInclusion ?? {}
    });
  }
}

export function setChapterNarration(chapterId: string, includeInNarration: boolean): void {
  manuscriptStore.update((state) => ({
    ...state,
    dirty: true,
    chapterInclusion: { ...state.chapterInclusion, [chapterId]: includeInNarration }
  }));
}
