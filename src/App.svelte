<script lang="ts">
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { buildChapterMergeInputs, deleteAudioFile, getChapterOutputPath, getExportOutputPath, mergeWavFiles, openAudioFile, revealAudioFile, selectExportPath, toAudioSrc, type AudioFileRecord } from "./lib/api/audioFiles";
  import { isTauriRuntime, openManuscriptPath, openManuscriptWithDialog, saveManuscriptFile } from "./lib/api/fileAccess";
  import { getSettingsFilePath, loadSettingsFile, saveSettingsFile, selectDirectory } from "./lib/api/settingsPersistence";
  import { ensureSidecar, restartSidecar, stopSidecar, type SidecarStatus } from "./lib/api/sidecarManager";
  import { openProjectPath, openProjectWithDialog, saveProjectWithDialog } from "./lib/project/projectPersistence";
  import type { ProjectSettings } from "./lib/project/projectTypes";
  import { buildPrompt, sourceTypeInstructions, type PromptOptions } from "./lib/prompt/promptTemplates";
  import { appSettingsStore, validateProjectSettings } from "./lib/stores/appSettings";
  import { generationLogsStore, generationStateStore, chunkStateStore, checkSidecar, clearChunkAudio, generateAll, generateChapter, logGeneration, regenerateChunk, regenerateFailedChunks, resetGenerationState, restoreChunkStates, stopGeneration } from "./lib/stores/generationQueue";
  import { manuscriptStore, markProjectSaved, markSaved, markSavedAs, reparseManuscript, restoreManuscriptProject, setChapterNarration, setManuscript, updateManuscript } from "./lib/stores/manuscriptStore";
  import { projectStore } from "./lib/stores/projectStore";
  import { recentFilesStore, rememberRecentFile } from "./lib/stores/recentFiles";
  import { recentProjectsStore, rememberRecentProject } from "./lib/stores/recentProjects";

  type ViewId = "home" | "manuscript" | "generation" | "audio" | "prompt" | "settings" | "logs";

  let activeView: ViewId = "home";
  let selectedChapterId = "";
  let promptOptions: PromptOptions = {
    sourceType: "PowerPoint / スライド資料",
    audience: "社内研修の受講者",
    style: "落ち着いた研修講師風",
    length: "15分程度",
    purpose: "重要ポイントを移動中に復習できる音声教材にする"
  };
  let copied = false;
  let fileError = "";
  let audioError = "";
  let selectedAudioPath = "";
  let chapterAudioPaths: Record<string, string> = {};
  let exportAudioPath = "";
  let nativeFileApi = isTauriRuntime();
  let sidecarStatus: SidecarStatus = "idle";
  let settingsFilePath = "";
  let settingsError = "";
  let settingsSavedMessage = "";

  $: chapters = $projectStore.chapters;
  $: if (!selectedChapterId && chapters.length > 0) selectedChapterId = chapters[0].id;
  $: selectedChapter = chapters.find((chapter) => chapter.id === selectedChapterId) ?? chapters[0];
  $: promptText = buildPrompt(promptOptions);
  $: progress = $generationStateStore.totalChunks > 0 ? Math.round(($generationStateStore.completedChunks / $generationStateStore.totalChunks) * 100) : 0;
  $: currentChunk = $generationStateStore.currentChunkId ? $chunkStateStore[$generationStateStore.currentChunkId] : undefined;
  $: failedChunks = Object.values($chunkStateStore).filter((chunk) => chunk.status === "failed");
  $: chunkAudioRecords = Object.values($chunkStateStore)
    .filter((chunk) => Boolean(chunk.audioPath))
    .map((chunk): AudioFileRecord => ({
      id: chunk.id,
      label: chunk.id,
      kind: "chunk",
      path: chunk.audioPath ?? "",
      chapterId: chunk.chapterId
    }));
  $: chapterAudioRecords = Object.entries(chapterAudioPaths).map(([chapterId, path]): AudioFileRecord => ({
    id: chapterId,
    label: chapters.find((chapter) => chapter.id === chapterId)?.title ?? chapterId,
    kind: "chapter",
    path,
    chapterId
  }));
  $: audioRecords = [
    ...chapterAudioRecords,
    ...(exportAudioPath ? [{ id: "export", label: "全体WAV", kind: "export" as const, path: exportAudioPath }] : []),
    ...chunkAudioRecords
  ];

  const navItems: Array<{ id: ViewId; label: string; icon: string }> = [
    { id: "home", label: "ホーム", icon: "⌂" },
    { id: "manuscript", label: "原稿", icon: "✎" },
    { id: "generation", label: "生成", icon: "▶" },
    { id: "audio", label: "音声", icon: "♪" },
    { id: "prompt", label: "プロンプト", icon: "⧉" },
    { id: "settings", label: "設定", icon: "⚙" },
    { id: "logs", label: "ログ", icon: "≡" }
  ];

  onMount(() => {
    loadPersistedSettings();
    if (!nativeFileApi) return;
    ensureSidecar({
      onStatus: (status) => (sidecarStatus = status),
      onLog: logGeneration
    }).catch((error) => {
      sidecarStatus = "failed";
      logGeneration("error", error instanceof Error ? error.message : String(error));
    });
    return () => {
      stopSidecar({ onLog: logGeneration }).catch((error) => {
        logGeneration("error", error instanceof Error ? error.message : String(error));
      });
    };
  });

  async function loadPersistedSettings(): Promise<void> {
    settingsError = "";
    if (!nativeFileApi) return;
    try {
      const loaded = await loadSettingsFile();
      settingsFilePath = loaded?.path ?? (await getSettingsFilePath()) ?? "";
      if (!loaded) return;
      appSettingsStore.set(loaded.settings);
      reparseManuscript();
      logGeneration("info", "settings.json を読み込みました。");
    } catch (error) {
      settingsError = error instanceof Error ? error.message : String(error);
      logGeneration("error", settingsError);
    }
  }

  async function startNativeSidecar(): Promise<boolean> {
    try {
      await ensureSidecar({
        onStatus: (status) => (sidecarStatus = status),
        onLog: logGeneration
      });
      return true;
    } catch (error) {
      sidecarStatus = "failed";
      logGeneration("error", error instanceof Error ? error.message : String(error));
      return false;
    }
  }

  async function restartNativeSidecar(): Promise<void> {
    try {
      await restartSidecar({
        onStatus: (status) => (sidecarStatus = status),
        onLog: logGeneration
      });
    } catch (error) {
      sidecarStatus = "failed";
      logGeneration("error", error instanceof Error ? error.message : String(error));
    }
  }

  async function generateAllWithSidecar(): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await generateAll();
  }

  async function generateChapterWithSidecar(chapterId: string): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await generateChapter(chapterId);
  }

  async function regenerateFailedWithSidecar(): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await regenerateFailedChunks();
  }

  async function regenerateChunkWithSidecar(chunkId: string): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await regenerateChunk(chunkId);
  }

  async function readFile(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    fileError = "";
    if (!/\.(md|txt)$/i.test(file.name)) {
      fileError = ".md または .txt の原稿を選択してください。";
      return;
    }
    setManuscript(await file.text(), file.name);
    activeView = "manuscript";
    input.value = "";
  }

  async function openNativeManuscript(): Promise<void> {
    fileError = "";
    try {
      const file = await openManuscriptWithDialog();
      if (!file) return;
      setManuscript(file.contents, file.name, file.path);
      rememberRecentFile(file.path, file.name);
      activeView = "manuscript";
    } catch (error) {
      fileError = error instanceof Error ? error.message : String(error);
    }
  }

  async function openRecent(path: string): Promise<void> {
    fileError = "";
    try {
      const file = await openManuscriptPath(path);
      if (!file) return;
      setManuscript(file.contents, file.name, file.path);
      rememberRecentFile(file.path, file.name);
      activeView = "manuscript";
    } catch (error) {
      fileError = error instanceof Error ? error.message : String(error);
    }
  }

  async function saveDraft(): Promise<void> {
    fileError = "";
    if (nativeFileApi) {
      try {
        const saved = await saveManuscriptFile(get(manuscriptStore).raw, get(manuscriptStore).filePath);
        if (saved) {
          markSavedAs(saved.name, saved.path);
          rememberRecentFile(saved.path, saved.name);
          return;
        }
      } catch (error) {
        fileError = error instanceof Error ? error.message : String(error);
        return;
      }
    }
    localStorage.setItem("koehon-studio-draft", get(manuscriptStore).raw);
    markSaved();
  }

  async function saveProject(): Promise<void> {
    fileError = "";
    if (!nativeFileApi) {
      fileError = "プロジェクト保存は Tauri アプリ上で利用できます。";
      return;
    }
    try {
      const manuscript = get(manuscriptStore);
      const saved = await saveProjectWithDialog(
        { ...get(projectStore), generation: get(generationStateStore) },
        manuscript.raw,
        manuscript.chapterInclusion,
        get(chunkStateStore),
        manuscript.projectFilePath
      );
      if (!saved) return;
      markProjectSaved(saved.projectDir, saved.projectFilePath, "manuscript.md", saved.manuscriptPath);
      rememberRecentProject(saved.projectFilePath, saved.snapshot.title);
      activeView = "manuscript";
    } catch (error) {
      fileError = error instanceof Error ? error.message : String(error);
    }
  }

  async function openProject(): Promise<void> {
    fileError = "";
    if (!nativeFileApi) {
      fileError = "プロジェクト読み込みは Tauri アプリ上で利用できます。";
      return;
    }
    try {
      const loaded = await openProjectWithDialog();
      if (!loaded) return;
      restoreLoadedProject(loaded);
    } catch (error) {
      fileError = error instanceof Error ? error.message : String(error);
    }
  }

  async function openRecentProject(path: string): Promise<void> {
    fileError = "";
    try {
      const loaded = await openProjectPath(path);
      restoreLoadedProject(loaded);
    } catch (error) {
      fileError = error instanceof Error ? error.message : String(error);
    }
  }

  function restoreLoadedProject(loaded: Awaited<ReturnType<typeof openProjectPath>>): void {
    appSettingsStore.set(loaded.snapshot.settings);
    restoreManuscriptProject(loaded.rawManuscript, {
      fileName: loaded.snapshot.manuscriptFile,
      filePath: loaded.manuscriptPath,
      projectDir: loaded.projectDir,
      projectFilePath: loaded.projectFilePath,
      chapterInclusion: loaded.snapshot.chapterInclusion
    });
    resetGenerationState(loaded.snapshot.generation);
    restoreChunkStates(loaded.snapshot.chunks, loaded.missingAudioPaths);
    rememberRecentProject(loaded.projectFilePath, loaded.snapshot.title);
    if (loaded.missingAudioPaths.length > 0) {
      logGeneration("error", `生成済み音声として記録された ${loaded.missingAudioPaths.length} 件のファイルが見つかりません。`);
    }
    activeView = "manuscript";
  }

  function loadDraft(): void {
    const draft = localStorage.getItem("koehon-studio-draft");
    if (draft) {
      setManuscript(draft, "local-draft.md");
      activeView = "manuscript";
    }
  }

  async function copyPrompt(): Promise<void> {
    await navigator.clipboard.writeText(promptText);
    copied = true;
    window.setTimeout(() => (copied = false), 1400);
  }

  function updateSetting<K extends keyof ProjectSettings>(key: K, value: ProjectSettings[K]): void {
    settingsSavedMessage = "";
    appSettingsStore.update((settings) => ({ ...settings, [key]: value }));
    settingsError = validateProjectSettings(get(appSettingsStore)).join("\n");
    reparseManuscript();
  }

  async function saveSettings(): Promise<void> {
    settingsError = validateProjectSettings(get(appSettingsStore)).join("\n");
    if (settingsError) return;
    try {
      const path = await saveSettingsFile(get(appSettingsStore));
      settingsFilePath = path ?? settingsFilePath;
      settingsSavedMessage = "設定を保存しました。";
      logGeneration("info", "settings.json を保存しました。");
    } catch (error) {
      settingsError = error instanceof Error ? error.message : String(error);
      logGeneration("error", settingsError);
    }
  }

  async function chooseModelDirectory(): Promise<void> {
    const selected = await selectDirectory("モデルディレクトリを選択");
    if (selected) updateSetting("modelDirectory", selected);
  }

  async function chooseOutputDirectory(): Promise<void> {
    const selected = await selectDirectory("出力ディレクトリを選択");
    if (selected) updateSetting("outputDirectory", selected);
  }

  function toggleChapterNarration(chapterId: string, includeInNarration: boolean): void {
    setChapterNarration(chapterId, includeInNarration);
  }

  async function mergeSelectedChapterAudio(): Promise<void> {
    if (!selectedChapter) return;
    await mergeChapterAudio(selectedChapter.id);
  }

  async function mergeChapterAudio(chapterId: string): Promise<string | undefined> {
    audioError = "";
    const chapter = get(projectStore).chapters.find((item) => item.id === chapterId);
    if (!chapter) return undefined;
    try {
      const project = get(projectStore);
      const outputPath = getChapterOutputPath(chapter, project.settings.outputDirectory || project.projectDir);
      const result = await mergeWavFiles(buildChapterMergeInputs(chapter, get(chunkStateStore)), outputPath);
      chapterAudioPaths = { ...chapterAudioPaths, [chapter.id]: result.outputPath };
      selectedAudioPath = result.outputPath;
      logGeneration("info", `${chapter.title} の章WAVを作成しました。`);
      return result.outputPath;
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
      logGeneration("error", audioError);
      return undefined;
    }
  }

  async function exportWholeWav(): Promise<void> {
    audioError = "";
    try {
      const project = get(projectStore);
      const targetChapters = project.chapters.filter((chapter) => chapter.includeInNarration);
      const chapterPaths: string[] = [];
      for (const chapter of targetChapters) {
        const path = chapterAudioPaths[chapter.id] ?? (await mergeChapterAudio(chapter.id));
        if (!path) return;
        chapterPaths.push(path);
      }
      const defaultPath = getExportOutputPath(project.title, project.settings.outputDirectory || project.projectDir);
      const outputPath = await selectExportPath(defaultPath);
      if (!outputPath) return;
      const result = await mergeWavFiles(chapterPaths.map((path) => ({ type: "file", path })), outputPath);
      exportAudioPath = result.outputPath;
      selectedAudioPath = result.outputPath;
      logGeneration("info", "全体WAVを書き出しました。");
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
      logGeneration("error", audioError);
    }
  }

  async function revealSelectedAudio(path: string): Promise<void> {
    try {
      await revealAudioFile(path);
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
    }
  }

  async function openSelectedAudio(path: string): Promise<void> {
    try {
      await openAudioFile(path);
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
    }
  }

  async function deleteGeneratedAudio(record: AudioFileRecord): Promise<void> {
    try {
      await deleteAudioFile(record.path);
      if (record.kind === "chunk") clearChunkAudio(record.id);
      if (record.kind === "chapter" && record.chapterId) {
        const remaining = { ...chapterAudioPaths };
        delete remaining[record.chapterId];
        chapterAudioPaths = remaining;
      }
      if (record.kind === "export") exportAudioPath = "";
      if (selectedAudioPath === record.path) selectedAudioPath = "";
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<svelte:window
  on:beforeunload={(event) => {
    if ($manuscriptStore.dirty) {
      event.preventDefault();
      event.returnValue = "";
    }
  }}
/>

<main class="shell">
  <aside class="sidebar" aria-label="メインナビゲーション">
    <div class="brand">
      <span class="brand-mark">K</span>
      <div>
        <strong>Koehon Studio</strong>
        <small>Local audiobook builder</small>
      </div>
    </div>
    <nav>
      {#each navItems as item}
        <button class:active={activeView === item.id} on:click={() => (activeView = item.id)} title={item.label}>
          <span aria-hidden="true">{item.icon}</span>
          {item.label}
        </button>
      {/each}
    </nav>
  </aside>

  <section class="workspace">
    <header class="topbar">
      <div>
        <h1>{$projectStore.title}</h1>
        <p>{chapters.length}章 / {chapters.flatMap((chapter) => chapter.chunks).length}チャンク</p>
      </div>
      <div class="actions">
        {#if nativeFileApi}
          <button class="primary" on:click={openNativeManuscript}>原稿を開く</button>
          <button on:click={openProject}>プロジェクトを開く</button>
          <button on:click={saveProject}>プロジェクト保存</button>
        {:else}
          <label class="file-button">
            原稿を開く
            <input type="file" accept=".md,.txt,text/markdown,text/plain" on:change={readFile} />
          </label>
        {/if}
      </div>
    </header>
    {#if fileError}<p class="error-banner">{fileError}</p>{/if}

    {#if activeView === "home"}
      <section class="panel home-grid">
        <div class="hero">
          <h2>AIで作ったMarkdown原稿を、章単位で音声化する作業台。</h2>
          <p>原稿を読み込み、章分割とチャンク分割を確認し、ローカルTTS sidecarへ順番に送信します。</p>
          <div class="actions">
            {#if nativeFileApi}
              <button class="primary" on:click={openNativeManuscript}>原稿を読み込む</button>
            {:else}
              <label class="primary-button">
                原稿を読み込む
                <input type="file" accept=".md,.txt,text/markdown,text/plain" on:change={readFile} />
              </label>
            {/if}
            <button on:click={loadDraft}>保存済み下書きを開く</button>
            {#if nativeFileApi}
              <button on:click={openProject}>プロジェクトを開く</button>
              <button on:click={saveProject}>プロジェクト保存</button>
            {/if}
            <button on:click={() => (activeView = "prompt")}>プロンプトを作る</button>
          </div>
        </div>
        <div class="stat-list">
          <div><strong>{$projectStore.metadata.source_type ?? "未設定"}</strong><span>元資料種別</span></div>
          <div><strong>{$projectStore.metadata.audience ?? "未設定"}</strong><span>対象読者</span></div>
          <div><strong>{$projectStore.metadata.language ?? "ja-JP"}</strong><span>言語</span></div>
          {#if $recentFilesStore.length > 0}
            <div class="recent-files">
              <strong>最近使った原稿</strong>
              {#each $recentFilesStore as file}
                <button disabled={!nativeFileApi} on:click={() => openRecent(file.path)} title={file.path}>{file.name}</button>
              {/each}
            </div>
          {/if}
          {#if $recentProjectsStore.length > 0}
            <div class="recent-files">
              <strong>最近使ったプロジェクト</strong>
              {#each $recentProjectsStore as project}
                <button disabled={!nativeFileApi} on:click={() => openRecentProject(project.path)} title={project.path}>{project.name}</button>
              {/each}
            </div>
          {/if}
        </div>
      </section>
    {:else if activeView === "manuscript"}
      <section class="manuscript-layout">
        <aside class="chapter-list">
          <div class="section-title">章</div>
          {#each chapters as chapter}
            <button class:active={selectedChapter?.id === chapter.id} class:muted={!chapter.includeInNarration} on:click={() => (selectedChapterId = chapter.id)}>
              <span>{String(chapter.order).padStart(2, "0")}</span>
              {chapter.title}
            </button>
          {/each}
        </aside>
        <div class="editor-column">
          <div class="toolbar">
            <span>{$manuscriptStore.fileName ?? "サンプル原稿"}</span>
            {#if $manuscriptStore.dirty}<em>未保存</em>{/if}
            <button on:click={saveDraft}>下書き保存</button>
            {#if nativeFileApi}<button on:click={saveProject}>プロジェクト保存</button>{/if}
          </div>
          <textarea value={$manuscriptStore.raw} on:input={(event) => updateManuscript((event.currentTarget as HTMLTextAreaElement).value)} spellcheck="false"></textarea>
        </div>
        <aside class="preview-column">
          <div class="section-title">選択章プレビュー</div>
          {#if selectedChapter}
            <h2>{selectedChapter.title}</h2>
            <label class="checkbox chapter-toggle">
              <input
                type="checkbox"
                checked={selectedChapter.includeInNarration}
                on:change={(event) => toggleChapterNarration(selectedChapter.id, (event.currentTarget as HTMLInputElement).checked)}
              />
              読み上げ対象
            </label>
            <pre>{selectedChapter.plainText}</pre>
            <div class="section-title">チャンク</div>
            <div class="chunk-list">
              {#each selectedChapter.chunks as chunk}
                <article class:pause={chunk.type === "pause"}>
                  <span>{chunk.order}</span>
                  {#if chunk.type === "pause"}
                    <strong>無音 {chunk.pauseMs}ms</strong>
                  {:else}
                    <p>{chunk.text}</p>
                  {/if}
                </article>
              {/each}
            </div>
          {/if}
        </aside>
      </section>
    {:else if activeView === "generation"}
      <section class="panel">
        <div class="generation-head">
          <div>
            <h2>音声生成</h2>
            <p>sidecar が起動している場合、textチャンクを順番に `/synthesize` へ送信します。</p>
          </div>
          <div class="actions">
            <button on:click={checkSidecar}>Health確認</button>
            <button on:click={startNativeSidecar}>Sidecar起動</button>
            <button on:click={restartNativeSidecar}>Sidecar再起動</button>
            <button class="primary" disabled={$generationStateStore.status === "running"} on:click={generateAllWithSidecar}>全体を生成</button>
            <button disabled={!selectedChapter || $generationStateStore.status === "running"} on:click={() => selectedChapter && generateChapterWithSidecar(selectedChapter.id)}>選択章を生成</button>
            <button disabled={failedChunks.length === 0 || $generationStateStore.status === "running"} on:click={regenerateFailedWithSidecar}>失敗のみ再生成</button>
            <button disabled={$generationStateStore.status !== "running"} on:click={stopGeneration}>停止</button>
          </div>
        </div>
        <div class="progress"><span style={`width: ${progress}%`}></span></div>
        <p>{progress}% / {$generationStateStore.status} / sidecar {sidecarStatus} / 完了 {$generationStateStore.completedChunks} / 失敗 {$generationStateStore.failedChunks}</p>
        {#if currentChunk}
          <div class="current-chunk">
            <strong>生成中: {currentChunk.id}</strong>
            <span>{currentChunk.text?.slice(0, 140) ?? `無音 ${currentChunk.pauseMs}ms`}</span>
          </div>
        {/if}
        <div class="chapter-progress">
          {#each chapters as chapter}
            {@const chapterChunks = chapter.chunks.map((chunk) => $chunkStateStore[chunk.id] ?? chunk)}
            {@const doneCount = chapterChunks.filter((chunk) => chunk.status === "done" || chunk.status === "skipped").length}
            {@const failedCount = chapterChunks.filter((chunk) => chunk.status === "failed").length}
            <article class:muted={!chapter.includeInNarration}>
              <strong>{chapter.title}</strong>
              <span>{doneCount}/{chapterChunks.length} 完了 / 失敗 {failedCount}</span>
              <button disabled={$generationStateStore.status === "running" || !chapter.includeInNarration} on:click={() => generateChapterWithSidecar(chapter.id)}>章を生成</button>
            </article>
          {/each}
        </div>
        <div class="queue-grid">
          {#each Object.values($chunkStateStore) as chunk}
            <article class={chunk.status}>
              <strong>{chunk.id}</strong>
              <span>{chunk.status}</span>
              <p>{chunk.type === "pause" ? `無音 ${chunk.pauseMs}ms` : chunk.text?.slice(0, 80)}</p>
              {#if chunk.error}<em>{chunk.error}</em>{/if}
              {#if chunk.status === "failed"}
                <button disabled={$generationStateStore.status === "running"} on:click={() => regenerateChunkWithSidecar(chunk.id)}>再生成</button>
              {/if}
            </article>
          {/each}
        </div>
      </section>
    {:else if activeView === "audio"}
      <section class="panel">
        <div class="generation-head">
          <div>
            <h2>音声プレビュー</h2>
            <p>生成済みチャンクを章WAVへ結合し、全体WAVとして書き出します。</p>
          </div>
          <div class="actions">
            <button disabled={!selectedChapter || !nativeFileApi} on:click={mergeSelectedChapterAudio}>選択章を結合</button>
            <button class="primary" disabled={chunkAudioRecords.length === 0 || !nativeFileApi} on:click={exportWholeWav}>全体WAVを書き出し</button>
          </div>
        </div>
        {#if audioError}<p class="error-banner">{audioError}</p>{/if}
        {#if selectedAudioPath}
          <div class="audio-player">
            <strong>{selectedAudioPath}</strong>
            <audio controls src={toAudioSrc(selectedAudioPath)}></audio>
            <div class="actions">
              <button on:click={() => openSelectedAudio(selectedAudioPath)}>開く</button>
              <button on:click={() => revealSelectedAudio(selectedAudioPath)}>フォルダで表示</button>
            </div>
          </div>
        {/if}
        <div class="audio-list">
          {#each audioRecords as record}
            <article class:active={selectedAudioPath === record.path}>
              <div>
                <strong>{record.label}</strong>
                <span>{record.kind} / {record.path}</span>
              </div>
              <div class="actions">
                <button on:click={() => (selectedAudioPath = record.path)}>再生</button>
                <button disabled={!nativeFileApi} on:click={() => revealSelectedAudio(record.path)}>表示</button>
                <button disabled={!nativeFileApi} on:click={() => deleteGeneratedAudio(record)}>削除</button>
              </div>
            </article>
          {/each}
        </div>
      </section>
    {:else if activeView === "prompt"}
      <section class="prompt-layout">
        <form class="panel controls">
          <label>元資料種別<select bind:value={promptOptions.sourceType}>{#each Object.keys(sourceTypeInstructions) as source}<option>{source}</option>{/each}</select></label>
          <label>対象読者<input bind:value={promptOptions.audience} /></label>
          <label>文体<input bind:value={promptOptions.style} /></label>
          <label>長さ<input bind:value={promptOptions.length} /></label>
          <label>目的<textarea bind:value={promptOptions.purpose}></textarea></label>
          <button type="button" class="primary" on:click={copyPrompt}>{copied ? "コピー済み" : "コピー"}</button>
        </form>
        <pre class="prompt-preview">{promptText}</pre>
      </section>
    {:else if activeView === "settings"}
      <section class="panel settings-panel">
        <div class="generation-head">
          <div>
            <h2>設定</h2>
            <p>{settingsFilePath || "ブラウザ実行中は localStorage に保存されます。"}</p>
          </div>
          <div class="actions">
            <button disabled={!nativeFileApi} on:click={loadPersistedSettings}>再読み込み</button>
            <button class="primary" disabled={!nativeFileApi || Boolean(settingsError)} on:click={saveSettings}>設定保存</button>
          </div>
        </div>
        {#if settingsError}<p class="error-banner">{settingsError}</p>{/if}
        {#if settingsSavedMessage}<p class="success-banner">{settingsSavedMessage}</p>{/if}
        <div class="settings-grid">
          <label>TTSエンジン<select value={$appSettingsStore.ttsEngine} disabled><option value="moss-tts-nano-onnx">MOSS-TTS-Nano ONNX</option></select></label>
          <label>CPUスレッド数<input type="number" min="1" max="32" value={$appSettingsStore.cpuThreads} on:input={(event) => updateSetting("cpuThreads", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>最大チャンク文字数<input type="number" min="100" max="1200" value={$appSettingsStore.maxChunkChars} on:input={(event) => updateSetting("maxChunkChars", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>サンプルレート(Hz)<input type="number" min="8000" max="192000" step="1000" value={$appSettingsStore.outputSampleRate} on:input={(event) => updateSetting("outputSampleRate", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>短い無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseShortMs} on:input={(event) => updateSetting("pauseShortMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>標準無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseMediumMs} on:input={(event) => updateSetting("pauseMediumMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>長い無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseLongMs} on:input={(event) => updateSetting("pauseLongMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          <label>既定話者<input value={$appSettingsStore.voice} on:input={(event) => updateSetting("voice", (event.currentTarget as HTMLInputElement).value)} /></label>
          <label>出力形式<select value={$appSettingsStore.exportFormat} disabled><option value="wav">wav</option></select></label>
          <label class="path-field">モデルディレクトリ<span><input value={$appSettingsStore.modelDirectory} on:input={(event) => updateSetting("modelDirectory", (event.currentTarget as HTMLInputElement).value)} /><button disabled={!nativeFileApi} on:click={chooseModelDirectory}>選択</button></span></label>
          <label class="path-field">出力ディレクトリ<span><input value={$appSettingsStore.outputDirectory} on:input={(event) => updateSetting("outputDirectory", (event.currentTarget as HTMLInputElement).value)} /><button disabled={!nativeFileApi} on:click={chooseOutputDirectory}>選択</button></span></label>
          <label class="checkbox"><input type="checkbox" checked={$appSettingsStore.includeManuscriptMemo} on:change={(event) => updateSetting("includeManuscriptMemo", (event.currentTarget as HTMLInputElement).checked)} /> 原稿作成メモを読み上げ対象にする</label>
        </div>
      </section>
    {:else if activeView === "logs"}
      <section class="panel log-list">
        {#each $generationLogsStore as log}
          <article class={log.level}><time>{log.at}</time><span>{log.message}</span></article>
        {/each}
      </section>
    {/if}
  </section>
</main>
