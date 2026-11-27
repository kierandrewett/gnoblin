<script lang="ts">
    // The import line of a class or a package, one for each language (shown by reference.css), and a Copy button that copies the shown one.
    import { copyText, type LanguageId } from "#lib/state.svelte.ts";
    import Icon from "../ui/Icon.svelte";

    let { text }: { text: Record<LanguageId, string> } = $props();
    let done = $state(false);
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function copy() {
        const lang = document.documentElement.dataset.lang as LanguageId | undefined;
        if (!(await copyText(text[lang && lang in text ? lang : "typescript"]))) return;
        done = true;
        clearTimeout(timer);
        timer = setTimeout(() => (done = false), 2000);
    }
</script>

<div class="ref-import">
    {#each Object.entries(text) as [lang, line] (lang)}<pre data-only={lang}><code>{line}</code></pre>{/each}
    <button type="button" class={["copy", done && "done"]} onclick={copy} aria-label="Copy the import line">
        <Icon name={done ? "check" : "copy"} size={15} />{done ? "Copied" : "Copy"}
    </button>
</div>
