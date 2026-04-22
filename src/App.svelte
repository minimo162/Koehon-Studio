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
  let notificationMessage = "";
  let notificationTone: "success" | "error" = "success";
  let notificationTimer: number | undefined;
  let isDragging = false;
  let chunkFilter: "all" | "pending" | "generating" | "done" | "failed" = "all";
  const shortcutModifier = typeof navigator !== "undefined" && /Mac|iPhone|iPad/i.test(navigator.platform) ? "⌘" : "Ctrl";
  let commandPaletteOpen = false;
  let commandQuery = "";
  let commandIndex = 0;
  let chapterQuery = "";
  let sidebarCollapsed = false;

  type Command = {
    id: string;
    label: string;
    group: string;
    hint?: string;
    run: () => void | Promise<void>;
    available?: () => boolean;
  };

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
  $: totalChunks = chapters.flatMap((chapter) => chapter.chunks).length;
  $: manuscriptLoaded = Boolean($manuscriptStore.raw && chapters.length > 0);
  $: filteredChunks = Object.values($chunkStateStore).filter((chunk) => chunkFilter === "all" || chunk.status === chunkFilter);
  $: filteredChapters = chapterQuery.trim() === ""
    ? chapters
    : chapters.filter((chapter) => chapter.title.toLowerCase().includes(chapterQuery.toLowerCase()));
  $: manuscriptCharCount = $manuscriptStore.raw?.length ?? 0;
  $: narrationCharCount = chapters
    .filter((chapter) => chapter.includeInNarration)
    .reduce((sum, chapter) => sum + (chapter.plainText?.length ?? 0), 0);
  $: estimatedReadingMinutes = Math.max(1, Math.round(narrationCharCount / 350));
  $: pauseChunkCount = chapters.flatMap((c) => c.chunks).filter((c) => c.type === "pause").length;
  $: commands = buildCommands();
  $: filteredCommands = (commandQuery.trim() === ""
    ? commands
    : commands.filter((c) => c.label.toLowerCase().includes(commandQuery.toLowerCase()) || c.group.includes(commandQuery))
  ).filter((c) => !c.available || c.available());
  $: groupedCommands = filteredCommands.reduce((acc, cmd) => {
    (acc[cmd.group] ??= []).push(cmd);
    return acc;
  }, {} as Record<string, Command[]>);
  $: if (commandIndex >= filteredCommands.length) commandIndex = Math.max(0, filteredCommands.length - 1);
  $: chunkStatusCounts = Object.values($chunkStateStore).reduce(
    (acc, chunk) => {
      acc.all += 1;
      acc[chunk.status] = (acc[chunk.status] ?? 0) + 1;
      return acc;
    },
    { all: 0, pending: 0, generating: 0, done: 0, failed: 0, skipped: 0 } as Record<string, number>
  );
  $: activeViewTitle = viewMeta[activeView].title;
  $: activeViewSubtitle = viewMeta[activeView].subtitle;
  $: activeViewStep = viewMeta[activeView].step;

  const workflowNav: Array<{ id: ViewId; label: string; idx: string }> = [
    { id: "home", label: "ホーム", idx: "01" },
    { id: "manuscript", label: "原稿", idx: "02" },
    { id: "generation", label: "生成", idx: "03" },
    { id: "audio", label: "音声", idx: "04" }
  ];

  const toolNav: Array<{ id: ViewId; label: string; idx: string }> = [
    { id: "prompt", label: "プロンプト", idx: "—" },
    { id: "settings", label: "設定", idx: "—" },
    { id: "logs", label: "ログ", idx: "—" }
  ];

  const viewMeta: Record<ViewId, { title: string; subtitle: string; step: string }> = {
    home: { title: "Koehon Studio", subtitle: "AIで作ったMarkdown原稿を、章ごとに音声化する作業台。", step: "はじめに" },
    manuscript: { title: "原稿", subtitle: "章分割とチャンク分割を確認し、読み上げ対象を整えます。", step: "02 原稿" },
    generation: { title: "音声生成", subtitle: "ローカルTTS sidecarへチャンクを順番に送り、音声を作ります。", step: "03 生成" },
    audio: { title: "音声プレビュー", subtitle: "章WAVへ結合し、最終的な全体WAVを書き出します。", step: "04 音声" },
    prompt: { title: "AI原稿プロンプト", subtitle: "外部AIへ貼り付ける、原稿生成用プロンプトを整形します。", step: "ツール" },
    settings: { title: "設定", subtitle: "TTS、無音、チャンク、出力に関する動作を調整します。", step: "ツール" },
    logs: { title: "ログ", subtitle: "sidecar、生成、書き出しの経緯を時系列で確認します。", step: "ツール" }
  };

  const sidecarLabel: Record<SidecarStatus, string> = {
    idle: "待機中",
    starting: "起動中",
    running: "接続済み",
    failed: "接続失敗",
    stopped: "停止済み"
  };

  function sidecarKind(status: SidecarStatus): string {
    if (status === "running") return "ready";
    if (status === "starting") return "starting";
    if (status === "failed") return "failed";
    return "idle";
  }

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
    notifyGenerationResult("全体生成");
  }

  async function generateChapterWithSidecar(chapterId: string): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await generateChapter(chapterId);
    notifyGenerationResult("章生成");
  }

  async function regenerateFailedWithSidecar(): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await regenerateFailedChunks();
    notifyGenerationResult("失敗チャンク再生成");
  }

  async function regenerateChunkWithSidecar(chunkId: string): Promise<void> {
    if (!(await startNativeSidecar())) return;
    await regenerateChunk(chunkId);
    notifyGenerationResult("チャンク再生成");
  }

  async function loadDroppedFile(file: File): Promise<void> {
    fileError = "";
    if (!/\.(md|txt)$/i.test(file.name)) {
      fileError = ".md または .txt の原稿を選択してください。";
      return;
    }
    setManuscript(await file.text(), file.name);
    activeView = "manuscript";
  }

  function buildCommands(): Command[] {
    const items: Command[] = [
      { id: "nav-home", label: "ホームへ移動", group: "移動", hint: "1", run: () => { activeView = "home"; } },
      { id: "nav-manuscript", label: "原稿へ移動", group: "移動", hint: "2", run: () => { activeView = "manuscript"; } },
      { id: "nav-generation", label: "生成へ移動", group: "移動", hint: "3", run: () => { activeView = "generation"; } },
      { id: "nav-audio", label: "音声へ移動", group: "移動", hint: "4", run: () => { activeView = "audio"; } },
      { id: "nav-prompt", label: "プロンプトへ移動", group: "移動", run: () => { activeView = "prompt"; } },
      { id: "nav-settings", label: "設定へ移動", group: "移動", run: () => { activeView = "settings"; } },
      { id: "nav-logs", label: "ログへ移動", group: "移動", run: () => { activeView = "logs"; } }
    ];
    if (nativeFileApi) {
      items.push(
        { id: "file-open", label: "原稿を開く", group: "ファイル", hint: `${shortcutModifier}O`, run: openNativeManuscript },
        { id: "project-open", label: "プロジェクトを開く", group: "ファイル", run: openProject },
        { id: "project-save", label: "プロジェクトを保存", group: "ファイル", hint: `${shortcutModifier}⇧S`, run: saveProject, available: () => manuscriptLoaded }
      );
    }
    items.push(
      { id: "draft-save", label: "下書きを保存", group: "ファイル", hint: `${shortcutModifier}S`, run: saveDraft },
      { id: "generate-all", label: "全体を生成", group: "生成", run: generateAllWithSidecar, available: () => manuscriptLoaded },
      { id: "generate-stop", label: "生成を停止", group: "生成", run: stopGeneration },
      { id: "generate-retry", label: "失敗のみ再生成", group: "生成", run: regenerateFailedWithSidecar, available: () => failedChunks.length > 0 },
      { id: "audio-export", label: "全体WAVを書き出し", group: "音声", run: exportWholeWav, available: () => chunkAudioRecords.length > 0 && nativeFileApi },
      { id: "prompt-copy", label: "プロンプトをコピー", group: "プロンプト", run: async () => { activeView = "prompt"; await copyPrompt(); } },
      { id: "sidebar-toggle", label: sidebarCollapsed ? "サイドバーを展開" : "サイドバーを畳む", group: "表示", run: () => { sidebarCollapsed = !sidebarCollapsed; } }
    );
    return items;
  }

  function openCommandPalette(): void {
    commandPaletteOpen = true;
    commandQuery = "";
    commandIndex = 0;
  }

  function closeCommandPalette(): void {
    commandPaletteOpen = false;
  }

  async function executeCommand(index: number): Promise<void> {
    const cmd = filteredCommands[index];
    if (!cmd) return;
    closeCommandPalette();
    await cmd.run();
  }

  function handlePaletteKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") { event.preventDefault(); closeCommandPalette(); return; }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      commandIndex = filteredCommands.length === 0 ? 0 : Math.min(commandIndex + 1, filteredCommands.length - 1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      commandIndex = Math.max(0, commandIndex - 1);
      return;
    }
    if (event.key === "Enter") { event.preventDefault(); void executeCommand(commandIndex); }
  }

  function handleKeydown(event: KeyboardEvent): void {
    const meta = event.metaKey || event.ctrlKey;
    if (meta && event.key.toLowerCase() === "k") {
      event.preventDefault();
      if (commandPaletteOpen) closeCommandPalette();
      else openCommandPalette();
      return;
    }
    if (commandPaletteOpen) return;
    if (!meta) return;
    const target = event.target as HTMLElement | null;
    const isEditable = target instanceof HTMLElement && (target.tagName === "TEXTAREA" || target.tagName === "INPUT");
    if (event.key.toLowerCase() === "o" && !isEditable) {
      event.preventDefault();
      if (nativeFileApi) openNativeManuscript();
    } else if (event.key.toLowerCase() === "s") {
      event.preventDefault();
      if (event.shiftKey && nativeFileApi) {
        saveProject();
      } else {
        saveDraft();
      }
    } else if (event.key === "1" && !isEditable) { event.preventDefault(); activeView = "home"; }
    else if (event.key === "2" && !isEditable) { event.preventDefault(); activeView = "manuscript"; }
    else if (event.key === "3" && !isEditable) { event.preventDefault(); activeView = "generation"; }
    else if (event.key === "4" && !isEditable) { event.preventDefault(); activeView = "audio"; }
    else if (event.key === "b" && !isEditable) { event.preventDefault(); sidebarCollapsed = !sidebarCollapsed; }
  }

  function handleDragOver(event: DragEvent): void {
    if (event.dataTransfer?.types?.includes("Files")) {
      event.preventDefault();
      isDragging = true;
    }
  }

  function handleDragLeave(event: DragEvent): void {
    if (!event.relatedTarget) isDragging = false;
  }

  async function handleDrop(event: DragEvent): Promise<void> {
    event.preventDefault();
    isDragging = false;
    const file = event.dataTransfer?.files?.[0];
    if (file) await loadDroppedFile(file);
  }

  function dismissNotification(): void {
    notificationMessage = "";
    if (notificationTimer) window.clearTimeout(notificationTimer);
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
      showNotification(`書き出しが完了しました: ${result.outputPath}`);
    } catch (error) {
      audioError = error instanceof Error ? error.message : String(error);
      logGeneration("error", audioError);
      showNotification(`書き出しに失敗しました: ${audioError}`, "error");
    }
  }

  function notifyGenerationResult(label: string): void {
    const state = get(generationStateStore);
    if (state.status === "completed") {
      showNotification(`${label}が完了しました。`);
      return;
    }
    if (state.status === "failed") {
      showNotification(`${label}が完了しましたが、失敗チャンクがあります。`, "error");
    }
  }

  function showNotification(message: string, tone: "success" | "error" = "success"): void {
    notificationMessage = message;
    notificationTone = tone;
    if (notificationTimer) window.clearTimeout(notificationTimer);
    notificationTimer = window.setTimeout(() => (notificationMessage = ""), 5000);
    void showSystemNotification(message);
  }

  async function showSystemNotification(message: string): Promise<void> {
    if (typeof Notification === "undefined") return;
    if (Notification.permission === "denied") return;
    const permission = Notification.permission === "granted" ? "granted" : await Notification.requestPermission();
    if (permission === "granted") new Notification("Koehon Studio", { body: message });
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
  on:keydown={handleKeydown}
  on:dragover={handleDragOver}
  on:dragleave={handleDragLeave}
  on:drop={handleDrop}
/>

{#if commandPaletteOpen}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="palette-backdrop" on:click={closeCommandPalette} on:keydown={handlePaletteKeydown}>
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="palette" role="dialog" aria-label="コマンドパレット" tabindex="-1" on:click|stopPropagation>
      <div class="palette-search">
        <span class="palette-kbd">⌘K</span>
        <!-- svelte-ignore a11y-autofocus -->
        <input
          autofocus
          bind:value={commandQuery}
          placeholder="コマンドを検索… (↑↓ で移動 · Enterで実行 · Esc)"
          on:keydown={handlePaletteKeydown}
        />
        <button class="ghost" on:click={closeCommandPalette}>閉じる</button>
      </div>
      {#if filteredCommands.length === 0}
        <div class="palette-empty">該当するコマンドがありません。</div>
      {:else}
        <div class="palette-list">
          {#each Object.entries(groupedCommands) as [groupName, cmds]}
            <div class="palette-group-label">{groupName}</div>
            {#each cmds as cmd}
              {@const globalIndex = filteredCommands.indexOf(cmd)}
              <button
                class="palette-item"
                class:active={globalIndex === commandIndex}
                on:mouseenter={() => (commandIndex = globalIndex)}
                on:click={() => executeCommand(globalIndex)}
              >
                <span class="palette-label">{cmd.label}</span>
                {#if cmd.hint}<kbd>{cmd.hint}</kbd>{/if}
              </button>
            {/each}
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if isDragging}
  <div class="drop-overlay" aria-hidden="true">
    <div class="drop-hint">
      <span class="drop-icon">原</span>
      <strong>ここに原稿をドロップ</strong>
      <small>.md · .txt</small>
    </div>
  </div>
{/if}

{#if notificationMessage}
  <div class="toast" class:toast-error={notificationTone === "error"} role="status">
    <span class="toast-bar" aria-hidden="true"></span>
    <div class="toast-body">
      <small>{notificationTone === "error" ? "エラー" : "完了"}</small>
      <span>{notificationMessage}</span>
    </div>
    <button class="toast-close" on:click={dismissNotification} aria-label="閉じる">×</button>
  </div>
{/if}

<main class="shell" class:sidebar-collapsed={sidebarCollapsed}>
  <aside class="sidebar" aria-label="メインナビゲーション">
    <div class="brand">
      <span class="brand-mark">声</span>
      <div class="brand-text">
        <strong>Koehon Studio</strong>
        <small>Audiobook Workbench</small>
      </div>
    </div>

    <button class="command-launcher" on:click={openCommandPalette} title="{shortcutModifier}+K">
      <span>コマンド…</span>
      <kbd>{shortcutModifier}K</kbd>
    </button>

    <div class="nav-section">
      <div class="nav-label">Workflow</div>
      {#each workflowNav as item}
        <button
          class="nav-item"
          class:active={activeView === item.id}
          on:click={() => (activeView = item.id)}
          title={item.label}
        >
          <span class="idx">{item.idx}</span>
          <span>{item.label}</span>
          <span class="nav-dot" aria-hidden="true"></span>
        </button>
      {/each}
    </div>

    <div class="nav-section">
      <div class="nav-label">Tools</div>
      {#each toolNav as item}
        <button
          class="nav-item"
          class:active={activeView === item.id}
          on:click={() => (activeView = item.id)}
          title={item.label}
        >
          <span class="idx">{item.idx}</span>
          <span>{item.label}</span>
          <span class="nav-dot" aria-hidden="true"></span>
        </button>
      {/each}
    </div>

    <div class="sidebar-footer">
      <div class="sidebar-status" data-status={sidecarKind(sidecarStatus)}>
        <span class="dot"></span>
        <span>TTS sidecar · {sidecarLabel[sidecarStatus]}</span>
      </div>
      <div>{nativeFileApi ? "Tauri native" : "Web preview"}</div>
    </div>
  </aside>

  <section class="workspace">
    <header class="topbar">
      <div class="topbar-text">
        <div class="eyebrow">
          <span>{activeViewStep}</span>
          {#if activeView !== "home" && $manuscriptStore.fileName}
            <span class="sep">／</span>
            <span>{$manuscriptStore.fileName}</span>
          {/if}
        </div>
        <h1>{activeView === "home" ? $projectStore.title : activeViewTitle}</h1>
        <p>{activeView === "home" ? activeViewSubtitle : `${chapters.length}章 · ${totalChunks}チャンク · ${activeViewSubtitle}`}</p>
      </div>
      <div class="actions">
        {#if nativeFileApi}
          <button class="primary with-kbd" on:click={openNativeManuscript} title="{shortcutModifier}+O">
            原稿を開く<kbd>{shortcutModifier}O</kbd>
          </button>
          <button on:click={openProject}>プロジェクトを開く</button>
          <button class="with-kbd" on:click={saveProject} disabled={!manuscriptLoaded} title="{shortcutModifier}+Shift+S">
            プロジェクト保存<kbd>{shortcutModifier}⇧S</kbd>
          </button>
        {:else}
          <label class="file-button primary">
            原稿を開く
            <input type="file" accept=".md,.txt,text/markdown,text/plain" on:change={readFile} />
          </label>
        {/if}
      </div>
    </header>

    {#if fileError}<p class="error-banner">{fileError}</p>{/if}

    {#key activeView}
    <div class="view-panel">
    {#if activeView === "home"}
      <section class="home">
        <div class="hero">
          <div class="hero-text">
            <div class="hero-eyebrow">Local audiobook workbench</div>
            <h2>AI原稿から、<em>静かな朗読</em>を。</h2>
            <p>外部AIで整えたMarkdown原稿を読み込み、章とチャンクを確認し、ローカルTTS sidecarで音声化します。本文・音声・プロジェクトはすべて手元で完結します。</p>
            <div class="hero-actions">
              {#if nativeFileApi}
                <button class="primary with-kbd" on:click={openNativeManuscript}>
                  原稿を読み込む<kbd>{shortcutModifier}O</kbd>
                </button>
                <button on:click={openProject}>プロジェクトを開く</button>
              {:else}
                <label class="primary-button">
                  原稿を読み込む
                  <input type="file" accept=".md,.txt,text/markdown,text/plain" on:change={readFile} />
                </label>
              {/if}
              <button class="ghost" on:click={() => (activeView = "prompt")}>プロンプトを作る →</button>
            </div>
          </div>
          <div class="hero-aside">
            <div class="meta-card">
              <small>元資料種別</small>
              <strong>{$projectStore.metadata.source_type ?? "未設定"}</strong>
            </div>
            <div class="meta-card">
              <small>対象読者</small>
              <strong>{$projectStore.metadata.audience ?? "未設定"}</strong>
            </div>
            <div class="meta-card">
              <small>言語</small>
              <strong>{$projectStore.metadata.language ?? "ja-JP"}</strong>
            </div>
          </div>
        </div>

        <div class="workflow">
          <article class="workflow-step">
            <div class="step-num">01</div>
            <h3>原稿を読み込む</h3>
            <p>外部AIが作ったMarkdownを取り込み、章とチャンクに自動分割します。front matterからタイトルや対象読者も抽出します。</p>
            <small>.md · .txt</small>
          </article>
          <article class="workflow-step">
            <div class="step-num">02</div>
            <h3>音声を生成する</h3>
            <p>章単位または全体で生成を開始。TTS sidecarがチャンクを順番に処理し、失敗したものだけを再生成できます。</p>
            <small>Local only · ONNX Runtime</small>
          </article>
          <article class="workflow-step">
            <div class="step-num">03</div>
            <h3>WAVを書き出す</h3>
            <p>pauseタグを無音として挿入し、章・全体のWAVを結合。ナレーションとして手元に残せます。</p>
            <small>WAV · 将来 MP3 / M4B</small>
          </article>
        </div>

        <div class="recent-grid">
          <div class="recent-card">
            <div class="section-title">最近使った原稿</div>
            <div class="recent-list">
              {#if $recentFilesStore.length === 0}
                <p class="empty">まだ原稿を開いていません。</p>
              {:else}
                {#each $recentFilesStore as file}
                  <button disabled={!nativeFileApi} on:click={() => openRecent(file.path)} title={file.path}>{file.name}</button>
                {/each}
              {/if}
            </div>
          </div>
          <div class="recent-card">
            <div class="section-title">最近使ったプロジェクト</div>
            <div class="recent-list">
              {#if $recentProjectsStore.length === 0}
                <p class="empty">プロジェクトを保存するとここに表示されます。</p>
              {:else}
                {#each $recentProjectsStore as project}
                  <button disabled={!nativeFileApi} on:click={() => openRecentProject(project.path)} title={project.path}>{project.name}</button>
                {/each}
              {/if}
            </div>
          </div>
        </div>
      </section>

    {:else if activeView === "manuscript"}
      {#if !manuscriptLoaded}
        <div class="empty-state">
          <div class="empty-icon">原</div>
          <h3>原稿がまだ読み込まれていません</h3>
          <p>Markdown(.md)またはテキスト(.txt)の原稿を開くと、章とチャンクが自動で整理されます。</p>
          <div class="actions">
            {#if nativeFileApi}
              <button class="primary" on:click={openNativeManuscript}>原稿を開く</button>
              <button on:click={openProject}>プロジェクトを開く</button>
            {:else}
              <label class="primary-button">
                原稿を開く
                <input type="file" accept=".md,.txt,text/markdown,text/plain" on:change={readFile} />
              </label>
            {/if}
          </div>
        </div>
      {:else}
        <section class="manuscript-layout">
          <aside class="chapter-list">
            <div class="chapter-list-head">
              <div class="section-title">章 · {chapters.length}</div>
              <div class="chapter-search">
                <input
                  type="search"
                  placeholder="章を検索…"
                  bind:value={chapterQuery}
                />
                {#if chapterQuery}
                  <button class="ghost" on:click={() => (chapterQuery = "")} aria-label="クリア">×</button>
                {/if}
              </div>
            </div>
            <div class="chapter-list-items">
              {#if filteredChapters.length === 0}
                <p class="chapter-empty">該当する章がありません。</p>
              {:else}
                {#each filteredChapters as chapter}
                  <button
                    class="chapter-item"
                    class:active={selectedChapter?.id === chapter.id}
                    class:muted={!chapter.includeInNarration}
                    on:click={() => (selectedChapterId = chapter.id)}
                  >
                    <span class="idx">{String(chapter.order).padStart(2, "0")}</span>
                    <span class="title-text">{chapter.title}</span>
                    <span class="chapter-count">{chapter.chunks.length}</span>
                  </button>
                {/each}
              {/if}
            </div>
          </aside>

          <div class="editor-column">
            <div class="toolbar">
              <span class="filename">{$manuscriptStore.fileName ?? "サンプル原稿"}</span>
              {#if $manuscriptStore.dirty}<em>未保存</em>{/if}
              <div class="toolbar-spacer"></div>
              <button on:click={saveDraft}>下書き保存</button>
              {#if nativeFileApi}<button class="primary" on:click={saveProject}>プロジェクト保存</button>{/if}
            </div>
            <div class="editor-stats">
              <span><strong>{manuscriptCharCount.toLocaleString()}</strong>文字</span>
              <span class="dot-sep">·</span>
              <span><strong>{chapters.length}</strong>章</span>
              <span class="dot-sep">·</span>
              <span><strong>{totalChunks}</strong>チャンク</span>
              {#if pauseChunkCount > 0}
                <span class="dot-sep">·</span>
                <span>無音 <strong>{pauseChunkCount}</strong></span>
              {/if}
              <span class="dot-sep">·</span>
              <span>読み上げ時間 <strong>~{estimatedReadingMinutes}</strong>分</span>
            </div>
            <textarea
              value={$manuscriptStore.raw}
              on:input={(event) => updateManuscript((event.currentTarget as HTMLTextAreaElement).value)}
              spellcheck="false"
              placeholder="ここに原稿が表示されます…"
            ></textarea>
          </div>

          <aside class="preview-column">
            <div class="section-title">選択章プレビュー</div>
            {#if selectedChapter}
              <h2>{selectedChapter.title}</h2>
              <label class="chapter-toggle">
                <input
                  type="checkbox"
                  checked={selectedChapter.includeInNarration}
                  on:change={(event) => toggleChapterNarration(selectedChapter.id, (event.currentTarget as HTMLInputElement).checked)}
                />
                読み上げ対象
              </label>
              <pre>{selectedChapter.plainText}</pre>
              <div class="section-title">チャンク · {selectedChapter.chunks.length}</div>
              <div class="chunk-list">
                {#each selectedChapter.chunks as chunk}
                  <article class:pause={chunk.type === "pause"}>
                    <span>{String(chunk.order).padStart(2, "0")}</span>
                    {#if chunk.type === "pause"}
                      <strong>無音 · {chunk.pauseMs}ms</strong>
                    {:else}
                      <p>{chunk.text}</p>
                    {/if}
                  </article>
                {/each}
              </div>
            {/if}
          </aside>
        </section>
      {/if}

    {:else if activeView === "generation"}
      {#if !manuscriptLoaded}
        <div class="empty-state">
          <div class="empty-icon">声</div>
          <h3>音声化する原稿がありません</h3>
          <p>まず「原稿」画面から原稿を読み込んでください。章分割とチャンク分割が完了すると生成できるようになります。</p>
          <button class="primary" on:click={() => (activeView = "manuscript")}>原稿を開く</button>
        </div>
      {:else}
        <section class="generation-layout">
          <div class="generation-head">
            <div>
              <h2>音声生成</h2>
              <p>textチャンクを順番に <code>/synthesize</code> へ送信します。pauseチャンクは無音として結合時に挿入されます。</p>
            </div>
            <div class="actions">
              <button on:click={checkSidecar}>Health確認</button>
              <button on:click={startNativeSidecar}>起動</button>
              <button on:click={restartNativeSidecar}>再起動</button>
            </div>
          </div>

          <div class="generation-meter" class:is-running={$generationStateStore.status === "running"}>
            <div class="meter-row">
              <div>
                <div class="meter-percent">{progress}%</div>
                <span class="status-chip {$generationStateStore.status}">
                  <span class="dot"></span>
                  {$generationStateStore.status}
                </span>
              </div>
              <div class="meter-stats">
                <div class="meter-stat">
                  <small>完了</small>
                  <strong>{$generationStateStore.completedChunks} / {$generationStateStore.totalChunks}</strong>
                </div>
                <div class="meter-stat fail">
                  <small>失敗</small>
                  <strong>{$generationStateStore.failedChunks}</strong>
                </div>
                <div class="meter-stat">
                  <small>Sidecar</small>
                  <strong>{sidecarLabel[sidecarStatus]}</strong>
                </div>
              </div>
            </div>
            <div class="progress"><span style={`width: ${progress}%`}></span></div>
            <div class="actions">
              <button class="primary" disabled={$generationStateStore.status === "running"} on:click={generateAllWithSidecar}>全体を生成</button>
              <button disabled={!selectedChapter || $generationStateStore.status === "running"} on:click={() => selectedChapter && generateChapterWithSidecar(selectedChapter.id)}>選択章を生成</button>
              <button class="danger" disabled={failedChunks.length === 0 || $generationStateStore.status === "running"} on:click={regenerateFailedWithSidecar}>失敗のみ再生成 ({failedChunks.length})</button>
              <button disabled={$generationStateStore.status !== "running"} on:click={stopGeneration}>停止</button>
            </div>
          </div>

          {#if currentChunk}
            <div class="current-chunk">
              <strong>NOW · {currentChunk.id}</strong>
              <span>{currentChunk.text?.slice(0, 140) ?? `無音 ${currentChunk.pauseMs}ms`}</span>
            </div>
          {/if}

          <div>
            <div class="section-title">章ごとの進捗</div>
            <div class="chapter-progress">
              {#each chapters as chapter}
                {@const chapterChunks = chapter.chunks.map((chunk) => $chunkStateStore[chunk.id] ?? chunk)}
                {@const doneCount = chapterChunks.filter((chunk) => chunk.status === "done" || chunk.status === "skipped").length}
                {@const failedCount = chapterChunks.filter((chunk) => chunk.status === "failed").length}
                <article class:muted={!chapter.includeInNarration}>
                  <strong>{chapter.title}</strong>
                  <span>{doneCount} / {chapterChunks.length} 完了{failedCount > 0 ? ` · 失敗 ${failedCount}` : ""}</span>
                  <button disabled={$generationStateStore.status === "running" || !chapter.includeInNarration} on:click={() => generateChapterWithSidecar(chapter.id)}>章を生成</button>
                </article>
              {/each}
            </div>
          </div>

          <div>
            <div class="filter-bar">
              <div class="section-title" style="margin:0">チャンク</div>
              <div class="filter-tabs" role="tablist">
                {#each [
                  { id: "all" as const, label: "すべて" },
                  { id: "pending" as const, label: "待機" },
                  { id: "generating" as const, label: "生成中" },
                  { id: "done" as const, label: "完了" },
                  { id: "failed" as const, label: "失敗" }
                ] as tab}
                  <button
                    class="filter-tab"
                    class:active={chunkFilter === tab.id}
                    on:click={() => (chunkFilter = tab.id)}
                  >
                    {tab.label}
                    <span class="filter-count">{chunkStatusCounts[tab.id] ?? 0}</span>
                  </button>
                {/each}
              </div>
            </div>
            {#if filteredChunks.length === 0}
              <p class="filter-empty">該当するチャンクはありません。</p>
            {:else}
              <div class="queue-grid">
                {#each filteredChunks as chunk}
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
            {/if}
          </div>
        </section>
      {/if}

    {:else if activeView === "audio"}
      <section class="generation-layout">
        <div class="generation-head">
          <div>
            <h2>音声プレビュー</h2>
            <p>生成済みチャンクを章WAVへ結合し、全体WAVとして書き出します。pauseは指定ミリ秒の無音として挿入されます。</p>
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

        {#if audioRecords.length === 0}
          <div class="empty-state">
            <div class="empty-icon">♪</div>
            <h3>まだ音声がありません</h3>
            <p>「生成」画面でチャンクを音声化すると、ここで再生・結合・書き出しができます。</p>
            <button class="primary" on:click={() => (activeView = "generation")}>生成画面へ</button>
          </div>
        {:else}
          <div>
            <div class="section-title">生成済みファイル · {audioRecords.length}</div>
            <div class="audio-list">
              {#each audioRecords as record}
                <article class:active={selectedAudioPath === record.path}>
                  <div>
                    <strong><span class="kind-badge">{record.kind}</span>{record.label}</strong>
                    <span>{record.path}</span>
                  </div>
                  <div class="actions">
                    <button on:click={() => (selectedAudioPath = record.path)}>再生</button>
                    <button disabled={!nativeFileApi} on:click={() => revealSelectedAudio(record.path)}>表示</button>
                    <button class="danger" disabled={!nativeFileApi} on:click={() => deleteGeneratedAudio(record)}>削除</button>
                  </div>
                </article>
              {/each}
            </div>
          </div>
        {/if}
      </section>

    {:else if activeView === "prompt"}
      <section class="prompt-layout">
        <form class="controls">
          <div class="section-title">プロンプト条件</div>
          <label>元資料種別<select bind:value={promptOptions.sourceType}>{#each Object.keys(sourceTypeInstructions) as source}<option>{source}</option>{/each}</select></label>
          <label>対象読者<input bind:value={promptOptions.audience} /></label>
          <label>文体<input bind:value={promptOptions.style} /></label>
          <label>長さ<input bind:value={promptOptions.length} /></label>
          <label>目的<textarea style="min-height:90px" bind:value={promptOptions.purpose}></textarea></label>
          <button type="button" class="primary" on:click={copyPrompt}>{copied ? "✓ コピー済み" : "プロンプトをコピー"}</button>
        </form>
        <pre class="prompt-preview">{promptText}</pre>
      </section>

    {:else if activeView === "settings"}
      <section class="settings-panel">
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

        <div class="settings-section">
          <header>
            <h3>TTSエンジン</h3>
            <p>推論に使うエンジン、話者、CPU並列数を指定します。</p>
          </header>
          <div class="settings-grid">
            <label>エンジン<select value={$appSettingsStore.ttsEngine} disabled><option value="moss-tts-nano-onnx">MOSS-TTS-Nano ONNX</option></select></label>
            <label>既定話者<input value={$appSettingsStore.voice} on:input={(event) => updateSetting("voice", (event.currentTarget as HTMLInputElement).value)} /></label>
            <label>CPUスレッド数<input type="number" min="1" max="32" value={$appSettingsStore.cpuThreads} on:input={(event) => updateSetting("cpuThreads", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          </div>
        </div>

        <div class="settings-section">
          <header>
            <h3>チャンク分割</h3>
            <p>TTSに渡す1チャンクの最大文字数。長すぎると不安定、短すぎると間が不自然になります。</p>
          </header>
          <div class="settings-grid">
            <label>最大チャンク文字数<input type="number" min="100" max="1200" value={$appSettingsStore.maxChunkChars} on:input={(event) => updateSetting("maxChunkChars", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          </div>
        </div>

        <div class="settings-section">
          <header>
            <h3>無音の長さ</h3>
            <p><code>[pause:short]</code>, <code>[pause:medium]</code>, <code>[pause:long]</code> タグに対応する無音の秒数(ms)。</p>
          </header>
          <div class="settings-grid">
            <label>短い無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseShortMs} on:input={(event) => updateSetting("pauseShortMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
            <label>標準無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseMediumMs} on:input={(event) => updateSetting("pauseMediumMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
            <label>長い無音(ms)<input type="number" min="0" max="10000" value={$appSettingsStore.pauseLongMs} on:input={(event) => updateSetting("pauseLongMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          </div>
        </div>

        <div class="settings-section">
          <header>
            <h3>出力</h3>
            <p>書き出すファイル形式とサンプルレート。</p>
          </header>
          <div class="settings-grid">
            <label>出力形式<select value={$appSettingsStore.exportFormat} disabled><option value="wav">WAV</option></select></label>
            <label>サンプルレート(Hz)<input type="number" min="8000" max="192000" step="1000" value={$appSettingsStore.outputSampleRate} on:input={(event) => updateSetting("outputSampleRate", Number((event.currentTarget as HTMLInputElement).value))} /></label>
          </div>
        </div>

        <div class="settings-section">
          <header>
            <h3>保存先</h3>
            <p>モデルと生成音声の配置ディレクトリ。</p>
          </header>
          <div class="settings-grid">
            <label class="path-field">モデルディレクトリ<span><input value={$appSettingsStore.modelDirectory} on:input={(event) => updateSetting("modelDirectory", (event.currentTarget as HTMLInputElement).value)} /><button disabled={!nativeFileApi} on:click={chooseModelDirectory}>選択</button></span></label>
            <label class="path-field">出力ディレクトリ<span><input value={$appSettingsStore.outputDirectory} on:input={(event) => updateSetting("outputDirectory", (event.currentTarget as HTMLInputElement).value)} /><button disabled={!nativeFileApi} on:click={chooseOutputDirectory}>選択</button></span></label>
          </div>
        </div>

        <div class="settings-section">
          <header>
            <h3>読み上げオプション</h3>
            <p>読み上げ対象に含めるかどうかの細かい調整。</p>
          </header>
          <div class="settings-grid">
            <label class="checkbox"><input type="checkbox" checked={$appSettingsStore.includeManuscriptMemo} on:change={(event) => updateSetting("includeManuscriptMemo", (event.currentTarget as HTMLInputElement).checked)} /> 原稿作成メモを読み上げ対象にする</label>
            <label class="checkbox"><input type="checkbox" checked={$appSettingsStore.readUrls} on:change={(event) => updateSetting("readUrls", (event.currentTarget as HTMLInputElement).checked)} /> URLを本文として読み上げる</label>
          </div>
        </div>
      </section>

    {:else if activeView === "logs"}
      <section class="log-list">
        {#if $generationLogsStore.length === 0}
          <p class="empty">ログはまだありません。</p>
        {:else}
          {#each $generationLogsStore as log}
            <article class={log.level}><time>{log.at}</time><span>{log.message}</span></article>
          {/each}
        {/if}
      </section>
    {/if}
    </div>
    {/key}
  </section>
</main>
