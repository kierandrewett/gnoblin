// Client state shared across the page: the search dialog and the polite live region for copy confirmations.

export const search = $state({ open: false });

export const announcer = $state({ message: "" });

/** Copies text and announces it in the live region (the layout renders `announcer.message`). */
export async function copyText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
    } catch {
        // Clipboard API unavailable (for example over plain HTTP): fall back to a selected textarea.
        const area = document.createElement("textarea");
        area.value = text;
        area.setAttribute("readonly", "");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.append(area);
        area.select();
        const ok = document.execCommand("copy");
        area.remove();
        if (!ok) return false;
    }
    announcer.message = "";
    queueMicrotask(() => (announcer.message = "Copied to the clipboard"));
    return true;
}
