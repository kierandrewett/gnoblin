<script lang="ts">
    import type { HTMLButtonAttributes } from "svelte/elements";
    import Icon from "./Icon.svelte";

    let { icon, label, ...rest }: { icon: string; label: string } & HTMLButtonAttributes = $props();
</script>

<button type="button" class="icon-btn" aria-label={label} {...rest}><Icon name={icon} size={20} /></button>

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

    @media (max-width: 1023px) {
        .icon-btn {
            width: 44px;
            height: 44px;
        }
    }
</style>
