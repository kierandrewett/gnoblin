<script lang="ts">
    // A code panel with a header (file name, language, Copy). `html` is the code highlighted by Shiki at build time
    // (highlightCode() in src/lib/server/markdown.ts); without it the code is shown plain. Code never wraps.
    import Icon from "../ui/Icon.svelte";
    import { codeBlocks } from "#lib/actions/code-blocks.ts";

    let {
        code,
        html = "",
        lang = "",
        file = "",
    }: { code: string; html?: string; lang?: string; file?: string } = $props();
</script>

<div class="code" use:codeBlocks={code}>
    <div class="code-head">
        {#if file}<span>{file}</span>{/if}
        <span class="lang">{lang}</span>
        <button type="button" class="copy" data-copy>
            <Icon name="copy" size={15} />
            <span>Copy</span>
        </button>
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <pre tabindex="0"><code
            >{#if html}{@html html}{:else}{code}{/if}</code
        ></pre>
</div>
