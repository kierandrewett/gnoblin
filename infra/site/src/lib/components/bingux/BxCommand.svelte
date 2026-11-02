<script lang="ts">
    // A Bingux field holding a command, with a Copy action button.
    import { copyText } from "#lib/state.svelte.ts";

    let { command }: { command: string } = $props();

    let copied = $state(false);
    let timer = 0;

    async function copy() {
        if (!(await copyText(command))) return;
        copied = true;
        window.clearTimeout(timer);
        timer = window.setTimeout(() => (copied = false), 2000);
    }
</script>

<div class="field">
    <svg class="ic" aria-hidden="true"><use href="/icons.svg#i-terminal" /></svg>
    <code class="fill cmd">{command}</code>
    <button type="button" class={["abtn", copied && "done"]} onclick={copy}>
        <svg class="ic" aria-hidden="true"><use href="/icons.svg#i-{copied ? 'check' : 'copy'}" /></svg>
        {copied ? "Copied" : "Copy"}
    </button>
</div>

<style>
    .field {
        padding: 2px;
        padding-left: var(--pad);
    }

    .cmd {
        font-family: var(--mono);
        font-size: var(--fs);
        line-height: 20px;
        overflow-wrap: anywhere;
        padding: 6px 0;
    }
</style>
