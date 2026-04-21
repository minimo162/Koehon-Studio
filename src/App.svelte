<script lang="ts">
  import { get } from "svelte/store";
  import { isTauriRuntime, openManuscriptPath, openManuscriptWithDialog, saveManuscriptFile } from "./lib/api/fileAccess";
  import { buildPrompt, sourceTypeInstructions, type PromptOptions } from "./lib/prompt/promptTemplates";
  import { appSettingsStore } from "./lib/stores/appSettings";
  import { generationLogsStore, generationStateStore, chunkStateStore, checkSidecar, generateAll, generateChapter, stopGeneration } from "./lib/stores/generationQueue";
  import { manuscriptStore, markSaved, markSavedAs, setChapterNarration, setManuscript, updateManuscript } from "./lib/stores/manuscriptStore";
  import { projectStore } from "./lib/stores/projectStore";
  import { recentFilesStore, rememberRecentFile } from "./lib/stores/recentFiles";

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
  let nativeFileApi = isTauriRuntime();

  $: chapters = $projectStore.chapters;
  $: if (!selectedChapterId && chapters.length > 0) selectedChapterId = chapters[0].id;
  $: selectedChapter = chapters.find((chapter) => chapter.id === selectedChapterId) ?? chapters[0];
  $: promptText = buildPrompt(promptOptions);
  $: progress = $generationStateStore.totalChunks > 0 ? Math.round(($generationStateStore.completedChunks / $generationStateStore.totalChunks) * 100) : 0;

  const navItems: Array<{ id: ViewId; label: string; icon: string }> = [
    { id: "home", label: "ホーム", icon: "⌂" },
    { id: "manuscript", label: "原稿", icon: "✎" },
    { id: "generation", label: "生成", icon: "▶" },
    { id: "audio", label: "音声", icon: "♪" },
    { id: "prompt", label: "プロンプト", icon: "⧉" },
    { id: "settings", label: "設定", icon: "⚙" },
    { id: "logs", label: "ログ", icon: "≡" }
  ];

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

  function updateSetting<K extends keyof typeof $appSettingsStore>(key: K, value: (typeof $appSettingsStore)[K]): void {
    appSettingsStore.update((settings) => ({ ...settings, [key]: value }));
    updateManuscript(get(manuscriptStore).raw);
  }

  function toggleChapterNarration(chapterId: string, includeInNarration: boolean): void {
    setChapterNarration(chapterId, includeInNarration);
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
            <button class="primary" disabled={$generationStateStore.status === "running"} on:click={generateAll}>全体を生成</button>
            <button disabled={!selectedChapter || $generationStateStore.status === "running"} on:click={() => selectedChapter && generateChapter(selectedChapter.id)}>選択章を生成</button>
            <button disabled={$generationStateStore.status !== "running"} on:click={stopGeneration}>停止</button>
          </div>
        </div>
        <div class="progress"><span style={`width: ${progress}%`}></span></div>
        <p>{progress}% / {$generationStateStore.status} / 失敗 {$generationStateStore.failedChunks}</p>
        <div class="queue-grid">
          {#each Object.values($chunkStateStore) as chunk}
            <article class={chunk.status}>
              <strong>{chunk.id}</strong>
              <span>{chunk.status}</span>
              <p>{chunk.type === "pause" ? `無音 ${chunk.pauseMs}ms` : chunk.text?.slice(0, 80)}</p>
              {#if chunk.error}<em>{chunk.error}</em>{/if}
            </article>
          {/each}
        </div>
      </section>
    {:else if activeView === "audio"}
      <section class="panel">
        <h2>音声プレビュー</h2>
        <p>生成済みチャンクのパスを一覧します。ブラウザ版ではローカルファイル再生権限が限定されるため、Tauri統合後にアプリ内再生を接続します。</p>
        <div class="queue-grid">
          {#each Object.values($chunkStateStore).filter((chunk) => chunk.audioPath) as chunk}
            <article>
              <strong>{chunk.id}</strong>
              <span>{chunk.audioPath}</span>
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
      <section class="panel settings-grid">
        <label>CPUスレッド数<input type="number" min="1" max="32" value={$appSettingsStore.cpuThreads} on:input={(event) => updateSetting("cpuThreads", Number((event.currentTarget as HTMLInputElement).value))} /></label>
        <label>最大チャンク文字数<input type="number" min="100" max="1200" value={$appSettingsStore.maxChunkChars} on:input={(event) => updateSetting("maxChunkChars", Number((event.currentTarget as HTMLInputElement).value))} /></label>
        <label>短い無音(ms)<input type="number" min="0" value={$appSettingsStore.pauseShortMs} on:input={(event) => updateSetting("pauseShortMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
        <label>標準無音(ms)<input type="number" min="0" value={$appSettingsStore.pauseMediumMs} on:input={(event) => updateSetting("pauseMediumMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
        <label>長い無音(ms)<input type="number" min="0" value={$appSettingsStore.pauseLongMs} on:input={(event) => updateSetting("pauseLongMs", Number((event.currentTarget as HTMLInputElement).value))} /></label>
        <label>既定話者<input value={$appSettingsStore.voice} on:input={(event) => updateSetting("voice", (event.currentTarget as HTMLInputElement).value)} /></label>
        <label>出力形式<select value={$appSettingsStore.exportFormat} on:change={(event) => updateSetting("exportFormat", (event.currentTarget as HTMLSelectElement).value as "wav" | "mp3" | "m4b")}><option>wav</option><option>mp3</option><option>m4b</option></select></label>
        <label class="checkbox"><input type="checkbox" checked={$appSettingsStore.includeManuscriptMemo} on:change={(event) => updateSetting("includeManuscriptMemo", (event.currentTarget as HTMLInputElement).checked)} /> 原稿作成メモを読み上げ対象にする</label>
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
