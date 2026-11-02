<script lang="ts">
    // Switches between light and dark and remembers the choice. app.html applies it before the first paint. The
    // icon and label for both schemes are in the page, and CSS shows the one that matches, so the prerendered
    // HTML is right whichever scheme the reader has.
    import Icon from "./Icon.svelte";

    function toggle() {
        const root = document.documentElement;
        const next = root.dataset.scheme === "dark" ? "light" : "dark";
        root.dataset.scheme = next;
        try {
            localStorage.setItem("gnoblin-scheme", next);
        } catch {
            // Storage blocked: the choice lasts for this page only.
        }
    }
</script>

<button type="button" class="icon-btn" onclick={toggle}>
    <span class="for-light"><Icon name="moon" size={20} /><span class="visually-hidden">Use dark appearance</span></span
    >
    <span class="for-dark"><Icon name="sun" size={20} /><span class="visually-hidden">Use light appearance</span></span>
</button>

<style>
    .icon-btn {
        width: 36px;
        height: 36px;
        display: inline-grid;
        place-items: center;
        border-radius: var(--r-pill);
        color: var(--ink-2);
        transition:
            background var(--dur-1) var(--ease),
            color var(--dur-1) var(--ease);
    }

    .icon-btn:hover {
        background: var(--sunken);
        color: var(--ink);
    }

    span:not(.visually-hidden) {
        display: inline-flex;
    }

    @media (max-width: 733px) {
        .icon-btn {
            width: 44px;
            height: 44px;
        }
    }
</style>
