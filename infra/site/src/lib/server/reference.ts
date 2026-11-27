// The reference model of the object model, read at build time from .generated/reference.json (scripts/reference.mjs writes it).
//
// Every name, type and declaration the reference pages show comes from that file. Nothing here spells a member: this module chooses what
// a page lists, links each type to the page that explains it, and writes each spelled thing once for each group of languages that spells
// it alike (src/lib/reference/html.ts). The pages are built from it in reference-pages.ts.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { esc, spell } from "../reference/html.ts";
import {
    LANGUAGE_IDS,
    type DefaultValue,
    type Declared,
    type Field,
    type LanguageId,
    type Method,
    type Property,
    type RawClass,
    type RawEnum,
    type RawNamespace,
    type RawReference,
    type Spelled,
    type TypeSpelling,
} from "../reference/types.ts";

export type Pkg = "core" | "std";
export const PKGS: Pkg[] = ["core", "std"];

/** What kind of thing a name is, and the page that explains it. */
export interface Entity {
    pkg: Pkg;
    name: string;
    kind: "class" | "service" | "record" | "interface" | "enum" | "callback";
    area: string;
    names: Spelled | null;
    href: string;
}

export interface Reference {
    raw: RawReference;
    namespaces: Record<Pkg, RawNamespace>;
    entities: Map<string, Entity>;
    classes: Map<string, RawClass>;
    enums: Map<string, RawEnum>;
    /** Names whose spelling differs by language, by schema name: what a description writes in backticks. */
    memberNames: Map<string, Spelled>;
    /** The meaning of each error code. */
    errorCodes: Map<string, string>;
}

export const key = (pkg: Pkg, name: string) => `${pkg}:${name}`;
export const slug = (name: string) => name.replace(/_/g, "-");

/** `gnoblin.model` is the core library and `gnoblin.std` the std library. */
function pkgOf(namespace: RawNamespace): Pkg {
    return namespace.name.endsWith(".model") ? "core" : "std";
}

let loaded: Reference | undefined;

export function reference(): Reference {
    if (loaded) return loaded;
    const file = __REFERENCE_JSON__;
    if (!existsSync(file)) {
        throw new Error(
            `${file} is missing. \`yarn reference\` (scripts/reference.mjs) writes it with the SDK generator: cargo xtask gen-sdks --target reference.`,
        );
    }
    const raw = JSON.parse(readFileSync(file, "utf8")) as RawReference;
    const namespaces = {} as Record<Pkg, RawNamespace>;
    for (const namespace of raw.namespaces) namespaces[pkgOf(namespace)] = namespace;
    for (const pkg of PKGS) {
        if (!namespaces[pkg]) throw new Error(`reference.json has no namespace for the ${pkg} library`);
    }

    const entities = new Map<string, Entity>();
    const classes = new Map<string, RawClass>();
    const enums = new Map<string, RawEnum>();
    const add = (entity: Entity) => {
        if (entities.has(key(entity.pkg, entity.name))) {
            throw new Error(`reference.json names ${entity.pkg} ${entity.name} twice`);
        }
        entities.set(key(entity.pkg, entity.name), entity);
    };
    for (const pkg of PKGS) {
        const namespace = namespaces[pkg];
        for (const c of namespace.classes) {
            classes.set(key(pkg, c.name), c);
            const page = c.kind === "handle" || c.kind === "service";
            add({
                pkg,
                name: c.name,
                kind: c.kind === "handle" ? "class" : c.kind,
                area: c.area,
                names: c.names,
                href: page ? `/reference/${pkg}/${slug(c.name)}` : `/reference/${pkg}/types/${slug(c.area)}#${c.name}`,
            });
        }
        for (const e of namespace.enums) {
            enums.set(key(pkg, e.name), e);
            add({
                pkg,
                name: e.name,
                kind: "enum",
                area: e.area,
                names: e.names,
                href: `/reference/${pkg}/types/${slug(e.area)}#${e.name}`,
            });
        }
        for (const cb of namespace.callbacks) {
            add({
                pkg,
                name: cb.name,
                kind: "callback",
                area: cb.area,
                names: null,
                href: `/reference/${pkg}/types/${slug(cb.area)}#${cb.name}`,
            });
        }
    }
    // A class page is /reference/<package>/<slug> and a types page /reference/<package>/types/<area>: no class may be called `types`.
    for (const pkg of PKGS) {
        if (entities.has(key(pkg, "types")))
            throw new Error(`a ${pkg} class is called "types", the folder of the types pages`);
    }
    loaded = {
        raw,
        namespaces,
        entities,
        classes,
        enums,
        memberNames: new Map(),
        errorCodes: readErrorCodes(),
    };
    loaded.memberNames = collectMemberNames(loaded);
    return loaded;
}

/** Resolves a name the schema writes (`client`, or `gnoblin.model.client` from the std library) from a class of `from`. */
export function resolve(from: Pkg, name: string): Entity | undefined {
    const { entities } = reference();
    if (name.startsWith("gnoblin.model.")) return entities.get(key("core", name.slice("gnoblin.model.".length)));
    if (name.startsWith("gnoblin.std.")) return entities.get(key("std", name.slice("gnoblin.std.".length)));
    return entities.get(key(from, name)) ?? entities.get(key("core", name));
}

/** The meaning of each error code: the table of docs/shell-api/errors.md, which the Rust `ErrorCode` points to for them. */
function readErrorCodes(): Map<string, string> {
    const file = path.join(__DOCS_DIR__, "shell-api/errors.md");
    const text = readFileSync(file, "utf8");
    const start = text.indexOf("\n## Error codes");
    if (start === -1) throw new Error(`${file} has no "Error codes" section`);
    const codes = new Map<string, string>();
    for (const row of text
        .slice(start + 1)
        .split("\n")
        .slice(1)) {
        const match = /^\|\s*`([a-z_]+)`\s*\|\s*(.*?)\s*\|\s*$/.exec(row);
        if (match) codes.set(match[1]!, match[2]!);
        else if (codes.size && !row.startsWith("|")) break;
    }
    if (!codes.size) throw new Error(`${file}: the "Error codes" section has no table of codes`);
    return codes;
}

/** Names whose spelling differs by language, to follow the code of a description to the language that is read. */
function collectMemberNames(data: Reference): Map<string, Spelled> {
    const found = new Map<string, Spelled | null>();
    const note = (name: string, names: Spelled) => {
        if (LANGUAGE_IDS.every((lang) => names[lang] === names.typescript)) return;
        const seen = found.get(name);
        if (seen === undefined) found.set(name, names);
        else if (seen && LANGUAGE_IDS.some((lang) => seen[lang] !== names[lang])) found.set(name, null);
    };
    for (const pkg of PKGS) {
        const namespace = data.namespaces[pkg];
        for (const list of [...namespace.classes, ...namespace.augments]) {
            for (const p of list.properties) note(p.name, p.names);
            for (const m of list.methods) note(m.name, m.names);
        }
        for (const c of namespace.classes) {
            for (const s of c.signals) {
                note(s.name, {
                    typescript: s.listener.typescript.event,
                    python: s.listener.python.event,
                    rust: s.listener.rust.method,
                });
            }
        }
    }
    const out = new Map<string, Spelled>();
    for (const [name, names] of found) if (names) out.set(name, names);
    return out;
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Text

const IDENT = /[A-Za-z_][A-Za-z0-9_]*/g;

/** The paragraphs of a schema description. A paragraph indented by two spaces more than the text around it is code. */
function paragraphs(text: string): { code: boolean; text: string }[] {
    const out: { code: boolean; text: string }[] = [];
    for (const block of text.trim().split(/\n\s*\n/)) {
        const lines = block.split("\n");
        const code = lines.length > 0 && lines.every((line) => line.startsWith("  "));
        const joined = code ? lines.map((l) => l.slice(2)).join("\n") : lines.map((l) => l.trim()).join(" ");
        if (joined.trim() !== "") out.push({ code, text: joined });
    }
    return out;
}

/** A span of code in a description, spelled the way each language spells the names in it. */
function codeSpan(code: string): string {
    const { memberNames } = reference();
    const html = {} as Record<LanguageId, string>;
    for (const lang of LANGUAGE_IDS)
        html[lang] = esc(code.replace(IDENT, (word) => memberNames.get(word)?.[lang] ?? word));
    return `<code>${spell(html)}</code>`;
}

/** Inline text with `code` spans. */
export function inline(text: string): string {
    return text
        .split(/(`[^`]+`)/)
        .map((part) =>
            part.length > 2 && part.startsWith("`") && part.endsWith("`") ? codeSpan(part.slice(1, -1)) : esc(part),
        )
        .join("");
}

const plainBlock = (code: string) => `<pre class="plain" tabindex="0"><code>${esc(code)}</code></pre>`;

/** The first sentence of a description. */
export function firstSentence(text: string): string {
    const first = paragraphs(text)[0];
    if (!first || first.code) return "";
    const match = /^(.*?[.!?])(\s|$)/.exec(first.text);
    return inline(match?.[1] ?? first.text);
}

/** What follows the first sentence of a description, as blocks. */
export function afterFirstSentence(text: string): string {
    const all = paragraphs(text);
    const first = all[0];
    if (!first) return "";
    const blocks: string[] = [];
    if (!first.code) {
        const rest = /^.*?[.!?]\s+(.*)$/.exec(first.text)?.[1];
        if (rest) blocks.push(`<p>${inline(rest)}</p>`);
    }
    for (const p of all.slice(1)) blocks.push(p.code ? plainBlock(p.text) : `<p>${inline(p.text)}</p>`);
    return blocks.join("");
}

/** A whole description as blocks. */
export function describe(text: string): string {
    return paragraphs(text)
        .map((p) => (p.code ? plainBlock(p.text) : `<p>${inline(p.text)}</p>`))
        .join("");
}

/** The first paragraph of a description, inline, and the paragraphs after it as blocks. */
export function split(text: string): { first: string; rest: string } {
    const all = paragraphs(text);
    const first = all[0];
    if (!first) return { first: "", rest: "" };
    return {
        first: first.code ? "" : inline(first.text),
        rest: all
            .slice(first.code ? 0 : 1)
            .map((p) => (p.code ? plainBlock(p.text) : `<p>${inline(p.text)}</p>`))
            .join(""),
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Types and declarations

export type Ref = { kind: string; name: string };

/** The classes, enums and callbacks a declaration may name: each language's name for them, with the page that explains it. */
function nameLinks(pkg: Pkg, refs: Ref[]): Map<LanguageId, Map<string, string>> {
    const out = new Map<LanguageId, Map<string, string>>();
    for (const lang of LANGUAGE_IDS) out.set(lang, new Map());
    for (const ref of refs) {
        const entity = resolve(pkg, ref.name);
        if (!entity?.names) continue;
        for (const lang of LANGUAGE_IDS) {
            const words = out.get(lang)!;
            if (!words.has(entity.names[lang])) words.set(entity.names[lang], entity.href);
        }
    }
    return out;
}

/** One line of a language's text with the names that have a page linked, and `bold` (a member's name) in bold, once. */
function linkLine(text: string, words: Map<string, string>, bold?: string, anchor?: string): string {
    let boldDone = bold === undefined;
    let out = "";
    let last = 0;
    for (const match of text.matchAll(IDENT)) {
        const word = match[0];
        out += esc(text.slice(last, match.index));
        last = match.index + word.length;
        const href = words.get(word);
        if (href) out += `<a class="ty" href="${href}">${esc(word)}</a>`;
        else if (!boldDone && word === bold) {
            out += anchor ? `<a class="anchor" href="#${anchor}"><b>${esc(word)}</b></a>` : `<b>${esc(word)}</b>`;
            boldDone = true;
        } else out += esc(word);
    }
    return out + esc(text.slice(last));
}

/** A type in each language, with its classes and enums linked. */
export function typeHtml(pkg: Pkg, type: TypeSpelling): string {
    const links = nameLinks(pkg, type.refs);
    const html = {} as Record<LanguageId, string>;
    for (const lang of LANGUAGE_IDS) html[lang] = linkLine(type[lang], links.get(lang)!);
    return spell(html);
}

/** The declaration lines of a member in each language, as the SDK files have them. */
export function declHtml(pkg: Pkg, declared: Declared, names: Spelled | null, refs: Ref[], anchor?: string): string {
    const links = nameLinks(pkg, refs);
    const html = {} as Record<LanguageId, string>;
    for (const lang of LANGUAGE_IDS) {
        html[lang] = declared[lang]
            .map((text) => `<span class="dl">${linkLine(text, links.get(lang)!, names?.[lang], anchor)}</span>`)
            .join("");
    }
    return spell(html);
}

/** The classes, enums and callbacks a member names. */
export const refsOf = (...types: (TypeSpelling | undefined)[]): Ref[] => types.flatMap((t) => t?.refs ?? []);

export function fieldRefs(fields: Field[]): Ref[] {
    return fields.flatMap((f) => f.type.refs);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Defaults, range and unit

function defaultText(value: DefaultValue, type: TypeSpelling, pkg: Pkg, lang: LanguageId): string {
    if (Array.isArray(value)) return `[${value.map((v) => defaultText(v, type, pkg, lang)).join(", ")}]`;
    if (typeof value === "boolean") return lang === "python" ? (value ? "True" : "False") : String(value);
    if (typeof value === "number") {
        // A whole number the schema types as a double is written `30.0` where the language tells a float from an int.
        const decimal = type.schema.includes("double") && Number.isInteger(value) && lang !== "typescript";
        return decimal ? `${value}.0` : String(value);
    }
    // A word of an enum that the type names is spelled the way that enum spells it.
    for (const ref of type.refs) {
        if (ref.kind !== "enum") continue;
        const entity = resolve(pkg, ref.name);
        const declared = entity && reference().enums.get(key(entity.pkg, entity.name));
        const member = declared?.values.find((v) => v.name === value);
        if (declared && member) {
            return lang === "typescript"
                ? member.names.typescript
                : `${declared.names[lang]}${lang === "python" ? "." : "::"}${member.names[lang]}`;
        }
    }
    return `"${value}"`;
}

/** The short lines under a description: the default, the range and the unit. */
export function facts(
    item: { default: DefaultValue | null; range: { min: number; max: number } | null; unit: string | null },
    type: TypeSpelling,
    pkg: Pkg,
): string[] {
    const out: string[] = [];
    if (item.default !== null) {
        const html = {} as Record<LanguageId, string>;
        for (const lang of LANGUAGE_IDS) html[lang] = `<code>${esc(defaultText(item.default, type, pkg, lang))}</code>`;
        out.push(`Default ${spell(html)}`);
    }
    if (item.range) out.push(`From ${item.range.min} to ${item.range.max}${item.unit ? ` ${esc(item.unit)}` : ""}`);
    else if (item.unit) out.push(`In ${esc(item.unit)}`);
    return out;
}

export type Member = Property | Method;
