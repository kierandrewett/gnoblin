// Behaviour for every code panel (.code) inside an element: the Copy button, and the fade on the right while a
// long line can still scroll. Works on markup from CodeBlock.svelte and from the Markdown renderer alike.
import type { Action } from "svelte/action";
import { copyText } from "../state.svelte.ts";

const COPIED_MS = 2000;

function setCopied(button: HTMLButtonElement, copied: boolean) {
    button.classList.toggle("done", copied);
    button.querySelector("use")?.setAttribute("href", `/icons.svg#i-${copied ? "check" : "copy"}`);
    const label = button.querySelector("span");
    if (label) label.textContent = copied ? "Copied" : "Copy";
}

function markOverflow(pre: HTMLElement) {
    const panel = pre.closest(".code");
    if (!panel) return;
    panel.classList.toggle("overflow", pre.scrollWidth > pre.clientWidth + 1);
    panel.classList.toggle("at-end", pre.scrollLeft + pre.clientWidth >= pre.scrollWidth - 1);
}

/** The parameter is the content (for example the page's HTML): when it changes, the panels are found again. */
export const codeBlocks: Action<HTMLElement, unknown> = (node) => {
    const timers = new WeakMap<HTMLButtonElement, number>();
    const resize = new ResizeObserver((entries) => {
        for (const entry of entries) markOverflow(entry.target as HTMLElement);
    });

    const onClick = async (event: MouseEvent) => {
        if (!(event.target instanceof Element)) return;
        const button = event.target.closest<HTMLButtonElement>("button[data-copy]");
        const pre = button?.closest(".code")?.querySelector("pre");
        if (!button || !pre || !node.contains(button)) return;
        if (!(await copyText(pre.textContent ?? ""))) return;
        setCopied(button, true);
        window.clearTimeout(timers.get(button));
        timers.set(
            button,
            window.setTimeout(() => setCopied(button, false), COPIED_MS),
        );
    };
    const onScroll = (event: Event) => {
        if (event.target instanceof HTMLElement && event.target.matches(".code pre")) markOverflow(event.target);
    };

    const scan = () => {
        resize.disconnect();
        for (const pre of node.querySelectorAll<HTMLElement>(".code pre")) resize.observe(pre);
    };

    node.addEventListener("click", onClick);
    node.addEventListener("scroll", onScroll, true);
    scan();
    return {
        update: scan,
        destroy() {
            node.removeEventListener("click", onClick);
            node.removeEventListener("scroll", onScroll, true);
            resize.disconnect();
        },
    };
};
