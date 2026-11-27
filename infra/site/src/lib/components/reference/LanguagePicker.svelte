<script lang="ts">
    // The language of every reference page: one radio group (one tab stop; the arrow keys move and choose) that sets the language of the
    // whole site, the same choice the guides' code tabs follow (src/lib/state.svelte.ts). It changes spelling only: a page holds every
    // language, and styles/reference.css shows one. The selected pill follows <html data-lang> in CSS, so it is right before this runs.
    import { onMount } from "svelte";
    import { LANGUAGE_MARKS } from "#lib/reference/language-marks.ts";
    import { LANGUAGE_EVENT, LANGUAGES, setLanguage, type LanguageId } from "#lib/state.svelte.ts";

    let current = $state<LanguageId>("typescript");
    let group: HTMLElement;

    onMount(() => {
        const read = () => {
            const lang = document.documentElement.dataset.lang;
            current = LANGUAGES.find((l) => l.id === lang)?.id ?? "typescript";
        };
        read();
        window.addEventListener(LANGUAGE_EVENT, read);
        return () => window.removeEventListener(LANGUAGE_EVENT, read);
    });

    function choose(lang: LanguageId) {
        current = lang;
        setLanguage(lang);
    }

    function onKey(event: KeyboardEvent) {
        const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
        if (step === undefined) return;
        event.preventDefault();
        const at = LANGUAGES.findIndex((l) => l.id === current);
        const next = LANGUAGES[(at + step + LANGUAGES.length) % LANGUAGES.length]!;
        choose(next.id);
        group.querySelector<HTMLElement>(`[data-lang="${next.id}"]`)?.focus();
    }
</script>

<!-- The buttons are the tab stop of the group (roving tabindex); the group itself takes none. -->
<!-- svelte-ignore a11y_interactive_supports_focus -->
<div class="lang-pick" role="radiogroup" aria-label="Language" bind:this={group} onkeydown={onKey}>
    {#each LANGUAGES as lang (lang.id)}
        <button
            type="button"
            role="radio"
            aria-checked={current === lang.id}
            tabindex={current === lang.id ? 0 : -1}
            data-lang={lang.id}
            onclick={() => choose(lang.id)}
        >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d={LANGUAGE_MARKS[lang.id]} /></svg>
            {lang.label}
        </button>
    {/each}
</div>
