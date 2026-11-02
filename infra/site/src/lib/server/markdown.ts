// Markdown to HTML for the docs pages, at build time. Supports what the pages in docs/ use, the way VitePress read
// them: GitHub-flavoured Markdown (tables), `::: details Title` and the callout containers (`::: tip` and so on),
// `{#id}` after a heading, VitePress heading ids, fenced code highlighted with Shiki, images from docs/ and links
// between pages written as `page.md`, `../page.md#anchor` or `/path#anchor`.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import type { Element, ElementContent, Root as HastRoot, Text } from "hast";
import { toString as hastToString } from "hast-util-to-string";
import type { Paragraph, Root as MdastRoot, RootContent } from "mdast";
import rehypeStringify from "rehype-stringify";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { createHighlighter, type Highlighter } from "shiki";
import { createCssVariablesTheme } from "shiki/core";
import { unified } from "unified";
import { visit } from "unist-util-visit";
import type { Heading } from "../docs.ts";

export interface RenderedPage {
    title: string;
    /** The paragraph right after the title, shown as the lede. */
    lede: string;
    body: string;
    headings: Heading[];
}

/** Resolves a link target written in a page to a site URL, or reports it as dead. */
export type LinkResolver = (target: string, fromFile: string) => string | null;

const LANGS = ["lua", "sh", "json", "nix", "javascript", "qml", "ini", "go", "glsl"] as const;
const LANG_LABELS: Record<string, string> = {
    lua: "Lua",
    sh: "Shell",
    json: "JSON",
    nix: "Nix",
    javascript: "JavaScript",
    qml: "QML",
    ini: "INI",
    go: "Go",
    glsl: "GLSL",
    text: "Text",
};
const CALLOUTS: Record<string, { icon: string; title: string }> = {
    info: { icon: "note", title: "Note" },
    note: { icon: "note", title: "Note" },
    tip: { icon: "tip", title: "Tip" },
    warning: { icon: "warning", title: "Warning" },
    danger: { icon: "danger", title: "Danger" },
};

let highlighter: Promise<Highlighter> | undefined;
function getHighlighter(): Promise<Highlighter> {
    // The css-variables theme writes var(--shiki-token-*) colours; content.css maps them to the --syn-* tokens, so
    // light and dark come from the site's own palette.
    if (!highlighter) {
        const theme = createCssVariablesTheme({ name: "gnoblin", variablePrefix: "--shiki-" });
        // Plain shell arguments and options are text: only the command and real strings get a colour.
        theme.tokenColors = [
            ...(theme.tokenColors ?? []),
            {
                scope: ["string.unquoted.argument.shell", "constant.other.option"],
                settings: { foreground: "var(--shiki-foreground)" },
            },
        ];
        highlighter = createHighlighter({ themes: [theme], langs: [...LANGS] });
    }
    return highlighter;
}

/** VitePress's heading slug: the same ids as the old site, so existing #anchors keep working. */
export function slugify(text: string): string {
    return (
        text
            .normalize("NFKD")
            .replace(/[\u0300-\u036F]/g, "")
            // eslint-disable-next-line no-control-regex -- VitePress removes control characters from slugs too.
            .replace(/[\u0000-\u001f]/g, "")
            .replace(/[\s~`!@#$%^&*()\-_+=[\]{}|\\;:"'“”‘’<>,.?/]+/g, "-")
            .replace(/-{2,}/g, "-")
            .replace(/^-+|-+$/g, "")
            .replace(/^(\d)/, "_$1")
            .toLowerCase()
    );
}

function el(tagName: string, properties: Element["properties"], children: ElementContent[] = []): Element {
    return { type: "element", tagName, properties, children };
}
const text = (value: string): Text => ({ type: "text", value });
const icon = (name: string, size = 16): Element =>
    el("svg", { className: ["icon"], width: size, height: size, ariaHidden: "true" }, [
        el("use", { href: `/icons.svg#i-${name}` }),
    ]);

/** Containers (`::: name title` ... `:::`) need a line of their own; give the markers blank lines around them so
 *  they parse as paragraphs, then turn the paragraphs into containers in remarkContainers. Fenced code is left
 *  alone. */
function separateContainerMarkers(source: string): string {
    let fence: string | null = null;
    const out: string[] = [];
    for (const line of source.split("\n")) {
        const open = /^\s*(`{3,}|~{3,})/.exec(line);
        if (open?.[1]) {
            if (fence === null) fence = open[1];
            else if (line.trim().startsWith(fence)) fence = null;
            out.push(line);
        } else if (fence === null && /^:::/.test(line.trim())) {
            out.push("", line.trim(), "");
        } else {
            out.push(line);
        }
    }
    return out.join("\n");
}

function markerOf(node: RootContent): string | null {
    if (node.type !== "paragraph") return null;
    const only = (node as Paragraph).children;
    if (only.length !== 1 || only[0]?.type !== "text") return null;
    const value = only[0].value;
    return value.startsWith(":::") ? value : null;
}

/** A node that remark-rehype turns into the element `hName` (finished in rehypeGnoblin). It holds block content,
 *  which mdast types do not allow inside a paragraph; remark-rehype converts any children, so the cast is safe. */
function container(hName: string, hProperties: Record<string, string>, children: RootContent[]): Paragraph {
    const blockChildren = children as Paragraph["children"];
    return { type: "paragraph", children: blockChildren, data: { hName, hProperties } };
}

/** Turns `::: details Title`, `::: tip` and friends into details and callout nodes. */
function remarkContainers() {
    return (tree: MdastRoot) => {
        const build = (nodes: RootContent[]): RootContent[] => {
            const out: RootContent[] = [];
            for (let i = 0; i < nodes.length; i++) {
                const node = nodes[i]!;
                const marker = markerOf(node);
                if (!marker || marker.trim() === ":::") {
                    if (marker) throw new Error("A ::: container closes without opening");
                    out.push(node);
                    continue;
                }
                const [, kind = "", title = ""] = /^:::\s*(\w+)\s*(.*)$/.exec(marker) ?? [];
                let depth = 1;
                let end = i + 1;
                for (; end < nodes.length; end++) {
                    const inner = markerOf(nodes[end]!);
                    if (!inner) continue;
                    if (inner.trim() === ":::") depth--;
                    else depth++;
                    if (depth === 0) break;
                }
                if (end >= nodes.length) throw new Error(`The container "${marker}" is not closed`);
                const children = build(nodes.slice(i + 1, end));
                i = end;
                if (kind === "details") {
                    out.push(container("details-container", { title: title || "Details" }, children));
                } else if (CALLOUTS[kind]) {
                    out.push(container("callout-container", { kind, title }, children));
                } else {
                    throw new Error(`Unknown container "::: ${kind}"`);
                }
            }
            return out;
        };
        tree.children = build(tree.children);
    };
}

/** Reads the size of a PNG, so images get width and height and do not shift the page while loading. */
function pngSize(file: string): { width: number; height: number } | null {
    if (!file.endsWith(".png") || !existsSync(file)) return null;
    const head = readFileSync(file).subarray(0, 24);
    return { width: head.readUInt32BE(16), height: head.readUInt32BE(20) };
}

interface RenderContext {
    file: string;
    docsDir: string;
    resolveLink: LinkResolver;
    errors: string[];
    headings: Heading[];
    highlighter: Highlighter;
}

function rehypeGnoblin(ctx: RenderContext) {
    return (tree: HastRoot) => {
        const used = new Map<string, number>();

        visit(tree, "element", (node, index, parent) => {
            if (!parent || index === undefined) return;

            // Headings: `{#id}` or the VitePress slug, made unique; h2 and h3 get an anchor and feed the outline.
            if (/^h[1-6]$/.test(node.tagName)) {
                const last = node.children.at(-1);
                let id: string | undefined;
                if (last?.type === "text") {
                    const m = /\s*\{#([\w-]+)\}\s*$/.exec(last.value);
                    if (m) {
                        id = m[1];
                        last.value = last.value.slice(0, m.index);
                    }
                }
                const label = hastToString(node).trim();
                if (!id) {
                    const base = slugify(label);
                    const n = used.get(base) ?? 0;
                    used.set(base, n + 1);
                    id = n === 0 ? base : `${base}-${n}`;
                }
                node.properties.id = id;
                if (node.tagName === "h2" || node.tagName === "h3") {
                    ctx.headings.push({ depth: node.tagName === "h2" ? 2 : 3, id, text: label });
                    node.children.unshift(
                        el("a", { className: ["anchor"], href: `#${id}`, ariaLabel: `Link to ${label}` }, [
                            icon("hash", 18),
                        ]),
                    );
                }
                return;
            }

            if (node.tagName === "a" && typeof node.properties.href === "string") {
                const href = node.properties.href;
                if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("#")) return;
                const resolved = ctx.resolveLink(href, ctx.file);
                if (resolved === null) ctx.errors.push(`${ctx.file}: dead link "${href}"`);
                else node.properties.href = resolved;
                return;
            }

            if (node.tagName === "img" && typeof node.properties.src === "string") {
                const src = node.properties.src;
                if (/^[a-z][a-z0-9+.-]*:/i.test(src)) return;
                const resolved = ctx.resolveLink(src, ctx.file);
                if (resolved === null) {
                    ctx.errors.push(`${ctx.file}: missing image "${src}"`);
                    return;
                }
                node.properties.src = resolved;
                const size = pngSize(path.join(ctx.docsDir, resolved.replace(/^\/docs\//, "")));
                if (size) Object.assign(node.properties, size);
                node.properties.loading = "lazy";
                node.properties.decoding = "async";
                return;
            }

            // Inline code (code outside pre).
            if (node.tagName === "code" && parent.type === "element" && parent.tagName !== "pre") {
                node.properties.className = ["inline"];
                return;
            }

            if (node.tagName === "pre") {
                const code = node.children.find((c): c is Element => c.type === "element" && c.tagName === "code");
                if (!code) return;
                const classes = (code.properties.className as string[] | undefined) ?? [];
                const lang = classes.find((c) => c.startsWith("language-"))?.slice("language-".length) ?? "";
                const source = hastToString(code).replace(/\n$/, "");
                const known = (LANGS as readonly string[]).includes(lang);
                if (lang && !known && lang !== "text") ctx.errors.push(`${ctx.file}: no highlighting for "${lang}"`);
                const highlighted = ctx.highlighter.codeToHast(source, {
                    lang: known ? lang : "text",
                    theme: "gnoblin",
                });
                const pre = highlighted.children[0] as Element;
                const head = el("div", { className: ["code-head"] }, [
                    el("span", { className: ["lang"] }, LANG_LABELS[lang] ? [text(LANG_LABELS[lang])] : []),
                    el("button", { type: "button", className: ["copy"], dataCopy: "" }, [
                        icon("copy", 15),
                        el("span", {}, [text("Copy")]),
                    ]),
                ]);
                const preOut = el("pre", { tabIndex: 0 }, pre.children);
                parent.children[index] = el("div", { className: ["code"] }, [head, preOut]);
                return "skip";
            }

            if (node.tagName === "table") {
                node.properties.className = ["tbl"];
                // Wrapped in place; the visit still goes on into the table's cells (links, code).
                parent.children[index] = el("div", { className: ["tbl-wrap"] }, [node]);
                return;
            }

            if (node.tagName === "details-container") {
                const title = String(node.properties.title ?? "Details");
                parent.children[index] = el("details", { className: ["details"] }, [
                    el("summary", {}, [
                        el("span", {}, [text(title)]),
                        el("span", { className: ["plus"] }, [icon("plus", 18)]),
                    ]),
                    el("div", { className: ["body"] }, node.children),
                ]);
                return;
            }

            if (node.tagName === "callout-container") {
                const kind = String(node.properties.kind);
                const meta = CALLOUTS[kind]!;
                const title = String(node.properties.title || meta.title);
                parent.children[index] = el(
                    "aside",
                    { className: ["callout", kind === "info" ? "note" : kind], role: "note" },
                    [
                        icon(meta.icon, 20),
                        el("div", {}, [el("p", { className: ["title"] }, [text(title)]), ...node.children]),
                    ],
                );
                return;
            }
            return;
        });
    };
}

export async function renderMarkdown(
    source: string,
    opts: { file: string; docsDir: string; resolveLink: LinkResolver },
): Promise<RenderedPage> {
    const ctx: RenderContext = {
        ...opts,
        errors: [],
        headings: [],
        highlighter: await getHighlighter(),
    };
    const processor = unified()
        .use(remarkParse)
        .use(remarkGfm)
        .use(remarkContainers)
        .use(remarkRehype)
        .use(rehypeGnoblin, ctx);
    const tree = (await processor.run(processor.parse(separateContainerMarkers(source)))) as HastRoot;
    if (ctx.errors.length) throw new Error(ctx.errors.join("\n"));

    // The title is the first h1; the paragraph right after it is the lede. Both are rendered by the page itself.
    const blocks = tree.children.filter((n) => !(n.type === "text" && n.value.trim() === ""));
    let title = "";
    let lede = "";
    const first = blocks[0];
    if (first?.type === "element" && first.tagName === "h1") {
        title = hastToString(first).trim();
        tree.children.splice(tree.children.indexOf(first), 1);
        const next = blocks[1];
        if (next?.type === "element" && next.tagName === "p") {
            lede = unified().use(rehypeStringify).stringify({ type: "root", children: next.children });
            tree.children.splice(tree.children.indexOf(next), 1);
        }
    }
    if (!title) throw new Error(`${opts.file}: the page has no "# Title" on its first line`);
    const body = unified().use(rehypeStringify).stringify(tree);
    return { title, lede, body, headings: ctx.headings };
}
