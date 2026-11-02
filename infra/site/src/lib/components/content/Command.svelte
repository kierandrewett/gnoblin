<script lang="ts">
    // One runnable command with a $ prompt (not copied) and Copy. "Copied" with a check for 2 s, announced politely.
    import Icon from "../ui/Icon.svelte";
    import { copyText } from "#lib/state.svelte.ts";

    let { command }: { command: string } = $props();

    let copied = $state(false);
    let timer = 0;
    // Kept as an expression: Svelte would trim the spaces of a literal " | " between elements.
    const PIPE = " | ";
    // Each command in a pipeline starts with the program name, shown in the function colour.
    const parts = $derived(
        command.split(" | ").map((part) => {
            const space = part.indexOf(" ");
            return space === -1 ? [part, ""] : [part.slice(0, space), part.slice(space)];
        }),
    );

    async function copy() {
        if (!(await copyText(command))) return;
        copied = true;
        window.clearTimeout(timer);
        timer = window.setTimeout(() => (copied = false), 2000);
    }
</script>

<div class="cmd">
    <code
        ><span class="p" aria-hidden="true">$</span>{#each parts as [program, rest], i (i)}{#if i}{PIPE}{/if}<span
                class="f">{program}</span
            >{rest}{/each}</code
    >
    <button type="button" class="copy" class:done={copied} onclick={copy}>
        <Icon name={copied ? "check" : "copy"} size={17} />
        <span>{copied ? "Copied" : "Copy"}</span>
    </button>
</div>
