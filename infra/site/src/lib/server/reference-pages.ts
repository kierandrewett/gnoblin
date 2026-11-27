// The pages of the API reference, built from the model in reference.ts: the index, a page for each class and service, a page of types for
// each area (records, interfaces, enums and callbacks), the errors page and the entries of the search index.
import { esc, only, spell, spelled } from "../reference/html.ts";
import type {
    AddedRow,
    ClassPage,
    Crumb,
    ErrorRow,
    ErrorsPage,
    EventRow,
    GlanceGroup,
    IndexPage,
    MemberLink,
    MethodRow,
    OutlineItem,
    PropertyRow,
    ReferenceNav,
    SearchRecord,
    TypeRow,
    TypesPage,
    TypesSection,
} from "../reference/model.ts";
import {
    LANGUAGE_IDS,
    type Example,
    type LanguageId,
    type Method,
    type Property,
    type RawAugment,
    type RawClass,
    type RawNamespace,
    type Signal,
} from "../reference/types.ts";
import { renderMarkdown } from "./markdown.ts";
import {
    PKGS,
    afterFirstSentence,
    declHtml,
    describe,
    facts,
    fieldRefs,
    firstSentence,
    inline,
    key,
    reference,
    refsOf,
    resolve,
    slug,
    split,
    typeHtml,
    type Pkg,
} from "./reference.ts";

/** What a package is, in a sentence. */
const BLURB: Record<Pkg, string> = {
    core: "The compositor's own objects: windows, outputs, input, the session and processes. A shell is complete on the core alone.",
    std: "The desktop above the core: workspaces, layouts, window policy and style, the scene and the widgets.",
};

const packageName = (pkg: Pkg) => reference().namespaces[pkg].package.typescript.package;
const areaTitle = (pkg: Pkg, id: string) => {
    const area = reference().namespaces[pkg].areas.find((a) => a.id === id);
    return area?.title ?? id;
};
const isPage = (c: RawClass) => c.kind === "handle" || c.kind === "service";

// ---------------------------------------------------------------------------------------------------------------------------------
// Markup helpers

interface Column {
    head: string;
    only?: LanguageId[];
    narrow?: boolean;
}

const att = (langs?: LanguageId[]) => (langs ? ` ${only(...langs)}` : "");

/** The width a column has by its heading (styles/reference.css): the first column and the type are narrow, the last takes the rest. */
const COLUMN_CLASS: Record<string, string> = {
    Name: "c-name",
    Parameter: "c-name",
    Value: "c-name",
    Code: "c-code",
    Type: "c-type",
    Flags: "c-flags",
};

/** A table. A cell of a language's own column carries `data-only`, so the table has the columns of the language that is shown. */
function table(label: string, columns: Column[], rows: { id?: string; cells: string[]; target?: boolean }[]): string {
    const head = columns
        .map(
            (c) =>
                `<th scope="col"${COLUMN_CLASS[c.head] ? ` class="${COLUMN_CLASS[c.head]}"` : ""}${att(c.only)}>${esc(c.head)}</th>`,
        )
        .join("");
    const body = rows
        .map((row) => {
            const cells = row.cells
                .map((cell, i) => {
                    const c = columns[i]!;
                    const label = ` data-label="${esc(c.head)}"`;
                    return i === 0
                        ? `<th scope="row"${att(c.only)}>${cell}</th>`
                        : `<td${att(c.only)}${label}>${cell}</td>`;
                })
                .join("");
            return `<tr${row.id ? ` id="${row.id}"` : ""}>${cells}</tr>`;
        })
        .join("");
    return `<div class="tbl-wrap" tabindex="0" role="region" aria-label="${esc(label)}"><table class="tbl ref-tbl"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
}

async function codeBlock(lang: LanguageId, source: string): Promise<string> {
    const page = await renderMarkdown(`# Example\n\n\`\`\`\`${lang}\n${source.replace(/\n+$/, "")}\n\`\`\`\`\n`, {
        file: "reference.json",
        docsDir: __DOCS_DIR__,
        resolveLink: () => null,
    });
    return page.body;
}

/** An example in the languages that have one. */
async function exampleHtml(example: Example | null): Promise<string> {
    if (!example) return "";
    const out: string[] = [];
    for (const lang of LANGUAGE_IDS) {
        const source = example[lang];
        if (source) out.push(`<div class="ex" ${only(lang)}>${await codeBlock(lang, source)}</div>`);
    }
    return out.join("");
}

function heading(level: 3 | 4, text: string): string {
    return `<h${level} class="sub">${esc(text)}</h${level}>`;
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Rows

function propertyRow(pkg: Pkg, p: Property): PropertyRow {
    const text = split(p.description);
    return {
        id: p.name,
        nameHtml: spelled(p.names),
        typeHtml: typeHtml(pkg, p.type),
        declHtml: declHtml(pkg, p.declarations, p.names, refsOf(p.type), p.name),
        access: p.readonly ? "read-only" : "read/write",
        animatable: p.animated,
        descHtml: text.first,
        moreHtml: text.rest,
        facts: facts(p, p.type, pkg),
    };
}

/** The properties of a record, listed as the fields of a parameter. */
function fieldsTable(pkg: Pkg, record: RawClass): string {
    return table(
        `Fields of ${record.names.typescript}`,
        [{ head: "Name" }, { head: "Type" }, { head: "Flags" }, { head: "Description" }],
        record.properties.map((p) => {
            const text = split(p.description);
            const extra = facts(p, p.type, pkg)
                .map((f) => `<span class="df">${f}</span>`)
                .join("");
            return {
                cells: [
                    `<b class="nm">${spelled(p.names)}</b>`,
                    typeHtml(pkg, p.type),
                    p.readonly ? "read-only" : "read/write",
                    `${text.first}${extra}`,
                ],
            };
        }),
    );
}

function enumValues(pkg: Pkg, name: string): string {
    const entity = resolve(pkg, name);
    const declared = entity && reference().enums.get(key(entity.pkg, entity.name));
    if (!declared) return "";
    return `<span class="vals">${declared.values.map((v) => `<code>${spelled(v.names)}</code>`).join(" ")}</span>`;
}

function parameterTable(pkg: Pkg, m: Method): string {
    return table(
        "Parameters",
        [{ head: "Parameter" }, { head: "Type" }, { head: "Notes" }],
        m.params.map((param) => {
            const text = split(param.description);
            const extra = facts(param, param.type, pkg)
                .map((f) => `<span class="df">${f}</span>`)
                .join("");
            const enums = param.type.refs
                .filter((r) => r.kind === "enum")
                .map((r) => enumValues(pkg, r.name))
                .join("");
            return {
                cells: [
                    `<b class="nm">${spelled(param.names)}</b>`,
                    typeHtml(pkg, param.type),
                    `${param.optional ? "Optional. " : "Required. "}${text.first}${extra}${enums}`,
                ],
            };
        }),
    );
}

/** The properties of a class and of the classes it extends, the base first: what an object of the class has. */
function withBases(pkg: Pkg, c: RawClass): Property[] {
    const base = c.base ? resolve(pkg, c.base) : undefined;
    const parent = base && reference().classes.get(key(base.pkg, base.name));
    return [...(parent ? withBases(base.pkg, parent) : []), ...c.properties];
}

async function methodBody(pkg: Pkg, m: Method): Promise<string> {
    const out: string[] = [];
    const rest = afterFirstSentence(m.description);
    if (rest) out.push(rest);
    if (m.params.length) {
        out.push(heading(4, "Parameters"), parameterTable(pkg, m));
        // A record the call takes opens in place with its fields; an enum lists its values (in the table above).
        for (const param of m.params) {
            for (const ref of param.type.refs) {
                const target = resolve(pkg, ref.name);
                const record = target && reference().classes.get(key(target.pkg, target.name));
                if (record?.kind === "record") {
                    out.push(
                        `<h4 class="sub">Fields of <a class="ty" href="${target!.href}">${spelled(record.names)}</a></h4>`,
                        fieldsTable(target!.pkg, record),
                    );
                }
            }
        }
    }
    if (m.returns) {
        out.push(heading(4, "Returns"));
        out.push(`<p class="ret">${typeHtml(pkg, m.returns.type)}${m.returns.optional ? "" : ""}</p>`);
        for (const ref of m.returns.type.refs) {
            const target = resolve(pkg, ref.name);
            const returned = target && reference().classes.get(key(target.pkg, target.name));
            if (m.operation && returned && (returned.kind === "handle" || returned.kind === "record")) {
                const base = returned.base ? resolve(target!.pkg, returned.base) : undefined;
                const extendsText = base?.names
                    ? ` <span class="mute">extends</span> <a class="ty" href="${base.href}">${spelled(base.names)}</a>`
                    : "";
                out.push(
                    `<p><a class="ty" href="${target!.href}">${spelled(returned.names)}</a>${extendsText}. ${firstSentence(returned.description)}</p>`,
                );
                out.push(
                    table(
                        `Members of ${returned.names.typescript}`,
                        [{ head: "Name" }, { head: "Type" }, { head: "Description" }],
                        withBases(target!.pkg, returned).map((p) => ({
                            cells: [
                                `<b class="nm">${spelled(p.names)}</b>`,
                                typeHtml(target!.pkg, p.type),
                                split(p.description).first,
                            ],
                        })),
                    ),
                );
            }
        }
    }
    if (m.raises.length) out.push(heading(4, "Errors"), errorTable(m.raises));
    const example = await exampleHtml(m.example);
    if (example) out.push(heading(4, "Example"), example);
    return out.join("");
}

function errorTable(codes: string[]): string {
    const { errorCodes } = reference();
    return table(
        "Errors",
        [{ head: "Code" }, { head: "Raised when" }],
        codes.map((code) => ({ cells: [`<b class="nm">${esc(code)}</b>`, inline(errorCodes.get(code) ?? "")] })),
    );
}

async function methodRow(pkg: Pkg, m: Method): Promise<MethodRow> {
    return {
        id: m.name,
        nameHtml: `${spelled(m.names)}()`,
        sigHtml: declHtml(pkg, m.declarations, m.names, [...refsOf(m.returns?.type), ...fieldRefs(m.params)]),
        summaryHtml: firstSentence(m.description),
        bodyHtml: await methodBody(pkg, m),
    };
}

function eventRow(pkg: Pkg, s: Signal): EventRow {
    const text = split(s.description);
    const name = {
        typescript: `"${s.listener.typescript.event}"`,
        python: `"${s.listener.python.event}"`,
        rust: s.listener.rust.method,
    };
    const args = {} as Record<LanguageId, string>;
    for (const lang of LANGUAGE_IDS) {
        args[lang] = s.payload.length
            ? lang === "rust"
                ? `(${s.payload.map((f) => esc(f.type.rust)).join(", ")})`
                : `(${s.payload.map((f) => `${esc(f.names[lang])}: ${esc(f.type[lang])}`).join(", ")})`
            : "()";
    }
    return {
        id: `event-${s.name}`,
        nameHtml: spelled(name),
        declHtml: declHtml(
            pkg,
            s.declarations,
            { typescript: "", python: "", rust: s.listener.rust.method },
            fieldRefs(s.payload),
            `event-${s.name}`,
        ),
        argsHtml: spell(args),
        descHtml: text.first,
        moreHtml: text.rest,
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Navigation

export function referenceNav(): ReferenceNav {
    const data = reference();
    return {
        packages: PKGS.map((pkg) => ({
            id: pkg,
            name: packageName(pkg),
            areas: data.namespaces[pkg].areas.map((area) => ({
                id: area.id,
                title: area.title ?? area.id,
                classes: area.classes
                    .map((name) => data.classes.get(key(pkg, name))!)
                    .filter(isPage)
                    .map((c) => ({ html: spelled(c.names), href: `/reference/${pkg}/${slug(c.name)}` })),
                types: hasTypes(pkg, area.id) ? `/reference/${pkg}/types/${slug(area.id)}` : null,
            })),
        })),
    };
}

function hasTypes(pkg: Pkg, areaId: string): boolean {
    const namespace = reference().namespaces[pkg];
    return (
        namespace.classes.some((c) => c.area === areaId && !isPage(c)) ||
        namespace.enums.some((e) => e.area === areaId) ||
        namespace.callbacks.some((c) => c.area === areaId)
    );
}

const REFERENCE_CRUMB: Crumb = { html: "Reference", href: "/reference" };

function crumbs(pkg: Pkg, areaId: string, last: Crumb): Crumb[] {
    return [
        REFERENCE_CRUMB,
        { html: esc(packageName(pkg)), href: `/reference#${pkg}` },
        { html: esc(areaTitle(pkg, areaId)), href: `/reference#${pkg}-${areaId}` },
        last,
    ];
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The index

export function indexPage(): IndexPage {
    const data = reference();
    return {
        errors: { codes: [...data.errorCodes.keys()].map((code) => ({ code })) },
        packages: PKGS.map((pkg) => {
            const namespace = data.namespaces[pkg];
            const sample = namespace.classes.find((c) => c.kind === "handle") ?? namespace.classes[0]!;
            const link = (c: RawClass) => ({ html: spelled(c.names), href: data.entities.get(key(pkg, c.name))!.href });
            return {
                id: pkg,
                name: packageName(pkg),
                blurb: BLURB[pkg],
                importHtml: spelled(sample.import),
                importText: sample.import,
                areas: namespace.areas.map((area) => {
                    const classes = area.classes.map((name) => data.classes.get(key(pkg, name))!);
                    const ofKind = (...kinds: string[]) => classes.filter((c) => kinds.includes(c.kind)).map(link);
                    const groups = [
                        { label: "Classes", links: ofKind("handle") },
                        { label: "Services", links: ofKind("service") },
                        { label: "Records", links: ofKind("record") },
                        { label: "Interfaces", links: ofKind("interface") },
                        {
                            label: "Enums",
                            links: area.enums.map((name) => {
                                const e = data.enums.get(key(pkg, name))!;
                                return { html: spelled(e.names), href: data.entities.get(key(pkg, name))!.href };
                            }),
                        },
                        {
                            label: "Callbacks",
                            links: area.callbacks.map((name) => ({
                                html: `<code>${esc(name)}</code>`,
                                href: data.entities.get(key(pkg, name))!.href,
                            })),
                        },
                    ].filter((g) => g.links.length);
                    return {
                        id: area.id,
                        title: area.title ?? area.id,
                        count: groups.reduce((n, g) => n + g.links.length, 0),
                        groups,
                    };
                }),
            };
        }),
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Class pages

export function classEntries(): { package: Pkg; name: string }[] {
    const out: { package: Pkg; name: string }[] = [];
    for (const pkg of PKGS) {
        for (const c of reference().namespaces[pkg].classes)
            if (isPage(c)) out.push({ package: pkg, name: slug(c.name) });
    }
    return out;
}

export function findClass(pkg: string, name: string): { pkg: Pkg; c: RawClass } | undefined {
    if (pkg !== "core" && pkg !== "std") return undefined;
    const c = reference().namespaces[pkg].classes.find((c) => isPage(c) && slug(c.name) === name);
    return c ? { pkg, c } : undefined;
}

/** What the other library adds to a core class. */
function augmentsOf(c: RawClass, pkg: Pkg): { from: Pkg; augment: RawAugment }[] {
    if (pkg !== "core") return [];
    const std = reference().namespaces.std;
    return std.augments.filter((a) => a.target === c.name).map((augment) => ({ from: "std" as Pkg, augment }));
}

/** The members a library adds to other classes, listed on the page of the area's own class. */
function addedElsewhere(c: RawClass, pkg: Pkg): AddedRow[] {
    const data = reference();
    const namespace: RawNamespace = data.namespaces[pkg];
    const area = namespace.areas.find((a) => a.id === c.area);
    if (!area || c.name !== area.id) return [];
    const out: AddedRow[] = [];
    for (const augment of namespace.augments.filter((a) => a.area === area.id)) {
        const target = data.classes.get(key("core", augment.target));
        const entity = data.entities.get(key("core", augment.target));
        if (!target || !entity) continue;
        for (const p of augment.properties) {
            out.push({
                id: `${augment.target}-${p.name}`,
                memberHtml: `${spelled(target.names)}.${spelled(p.names)}`,
                memberHref: `${entity.href}#${p.name}`,
                typeHtml: typeHtml(pkg, p.type),
                declHtml: declHtml(pkg, p.declarations, p.names, refsOf(p.type)),
                descHtml: firstSentence(p.description),
            });
        }
        for (const m of augment.methods) {
            out.push({
                id: `${augment.target}-${m.name}`,
                memberHtml: `${spelled(target.names)}.${spelled(m.names)}()`,
                memberHref: `${entity.href}#${m.name}`,
                typeHtml: declHtml(pkg, m.declarations, m.names, [...refsOf(m.returns?.type), ...fieldRefs(m.params)]),
                declHtml: declHtml(pkg, m.declarations, m.names, [...refsOf(m.returns?.type), ...fieldRefs(m.params)]),
                descHtml: firstSentence(m.description),
            });
        }
    }
    return out;
}

/** The enums and records the members of a class name: the types a reader meets on this page. */
function typesUsed(c: RawClass, pkg: Pkg, extra: { properties: Property[]; methods: Method[] }[]): TypeRow[] {
    const data = reference();
    const refs = [c, ...extra].flatMap((m) => [
        ...m.properties.flatMap((p) => p.type.refs),
        ...m.methods.flatMap((x) => [...fieldRefs(x.params), ...refsOf(x.returns?.type)]),
    ]);
    const seen = new Set<string>();
    const rows: TypeRow[] = [];
    for (const ref of refs) {
        const entity = resolve(pkg, ref.name);
        if (!entity || seen.has(key(entity.pkg, entity.name))) continue;
        seen.add(key(entity.pkg, entity.name));
        if (entity.kind === "enum") {
            const e = data.enums.get(key(entity.pkg, entity.name))!;
            rows.push({
                id: e.name,
                nameHtml: spelled(e.names),
                href: entity.href,
                kind: "enum",
                valuesHtml: e.values.map((v) => `<code>${spelled(v.names)}</code>`).join(" "),
                descHtml: firstSentence(e.description),
            });
        } else if (entity.kind === "record") {
            const r = data.classes.get(key(entity.pkg, entity.name))!;
            const fields = r.properties.map((p) => {
                const parts = {} as Record<LanguageId, string>;
                for (const lang of LANGUAGE_IDS) {
                    parts[lang] = esc(
                        lang === "rust" ? `${p.names.rust}: ${p.type.rust}` : `${p.names[lang]}: ${p.type[lang]}`,
                    );
                }
                return `<code>${spell(parts)}</code>`;
            });
            rows.push({
                id: r.name,
                nameHtml: spelled(r.names),
                href: entity.href,
                kind: "record",
                valuesHtml: fields.join(" "),
                descHtml: firstSentence(r.description),
            });
        }
    }
    return rows;
}

const ERROR_PROSE: Record<LanguageId, string> = {
    typescript:
        "In TypeScript, a call that cannot be made throws <code>GnoblinError</code> with <code>code</code> and <code>target</code>. A value of the wrong type is a <code>TypeError</code>, not a <code>GnoblinError</code>.",
    python: "In Python, a call that cannot be made raises <code>GnoblinError</code> with <code>code</code> and <code>target</code>. A value of the wrong type is a <code>TypeError</code>, not a <code>GnoblinError</code>.",
    rust: "In Rust, a call that cannot be made returns <code>Err(Fail)</code>: <code>fail.code()</code> and <code>fail.target()</code> say what failed.",
};

/** Every property or method, in any class or augment, whose type is the class: how a reader gets one. */
function returnedBy(pkg: Pkg, c: RawClass): MemberLink[] {
    const data = reference();
    const out: MemberLink[] = [];
    const note = (owner: RawClass, ownerHref: string, member: Property | Method) => {
        const type = "type" in member ? member.type : member.returns?.type;
        if (!type) return;
        const match = type.refs.some((r) => {
            const target = resolve(owner === c ? pkg : ownerPkg(owner), r.name);
            return r.kind === "class" && target?.pkg === pkg && target.name === c.name;
        });
        if (!match) return;
        out.push({
            html: `${spelled(owner.names)}.${spelled(member.names)}${"type" in member ? "" : "()"}`,
            href: `${ownerHref}#${member.name}`,
        });
    };
    for (const p of PKGS) {
        const namespace = data.namespaces[p];
        for (const owner of namespace.classes.filter(isPage)) {
            const href = data.entities.get(key(p, owner.name))!.href;
            for (const m of owner.properties) note(owner, href, m);
            for (const m of owner.methods) note(owner, href, m);
        }
        for (const augment of namespace.augments) {
            const owner = data.classes.get(key("core", augment.target));
            const href = data.entities.get(key("core", augment.target))?.href;
            if (!owner || !href) continue;
            for (const m of augment.properties) note(owner, href, m);
            for (const m of augment.methods) note(owner, href, m);
        }
    }
    return out;
}

function ownerPkg(owner: RawClass): Pkg {
    return reference().namespaces.core.classes.includes(owner) ? "core" : "std";
}

/** The members that raise a code, as many as a cell holds, and how many more there are. */
const BY_SHOWN = 6;
const shown = (links: string[]) => ({ by: links.slice(0, BY_SHOWN), more: Math.max(0, links.length - BY_SHOWN) });

/** The codes the members of a page raise, and which members raise each. */
function errorRows(
    pkg: Pkg,
    members: { name: string; names: Property["names"]; raises: string[]; method: boolean }[],
): ErrorRow[] {
    const { errorCodes } = reference();
    const by = new Map<string, string[]>();
    for (const m of members) {
        for (const code of m.raises) {
            const list = by.get(code) ?? [];
            list.push(`<a href="#${m.name}">${spelled(m.names)}${m.method ? "()" : ""}</a>`);
            by.set(code, list);
        }
    }
    void pkg;
    return [...errorCodes.keys()]
        .filter((code) => by.has(code))
        .map((code) => ({ code, meaning: inline(errorCodes.get(code)!), ...shown(by.get(code)!) }));
}

export async function classPage(pkg: Pkg, c: RawClass): Promise<ClassPage> {
    const added = augmentsOf(c, pkg);

    // Everything with an anchor: a name may be used once on a page.
    const ids = new Set<string>();
    const unique = <T extends { id: string }>(rows: T[]): T[] => {
        for (const row of rows) {
            if (ids.has(row.id)) throw new Error(`${pkg} ${c.name}: the anchor "#${row.id}" is used twice`);
            ids.add(row.id);
        }
        return rows;
    };

    const properties = unique(c.properties.map((p) => propertyRow(pkg, p)));
    const methods = unique(await Promise.all(c.methods.map((m) => methodRow(pkg, m))));
    const events = unique(c.signals.map((s) => eventRow(pkg, s)));
    const addedProperties = unique(added.flatMap((a) => a.augment.properties.map((p) => propertyRow(a.from, p))));
    const addedMethods = unique(
        await Promise.all(added.flatMap((a) => a.augment.methods.map((m) => methodRow(a.from, m)))),
    );

    const returned = returnedBy(pkg, c);
    const shownReturned = returned.slice(0, 4);
    const everyMember = [
        ...c.properties.map((p) => ({ name: p.name, names: p.names, raises: p.raises, method: false })),
        ...c.methods.map((m) => ({ name: m.name, names: m.names, raises: m.raises, method: true })),
        ...added.flatMap((a) => [
            ...a.augment.properties.map((p) => ({ name: p.name, names: p.names, raises: p.raises, method: false })),
            ...a.augment.methods.map((m) => ({ name: m.name, names: m.names, raises: m.raises, method: true })),
        ]),
    ];
    const errors = errorRows(pkg, everyMember);
    const elsewhere = addedElsewhere(c, pkg);
    const types = typesUsed(
        c,
        pkg,
        added.map((a) => a.augment),
    );
    const example = await exampleHtml(c.example);
    const base = c.base ? resolve(pkg, c.base) : undefined;

    const link = (html: string, id: string): MemberLink => ({ html, href: `#${id}` });
    const glance: GlanceGroup[] = [
        { label: "Properties", links: properties.map((r) => link(r.nameHtml, r.id)) },
        { label: "Methods", links: methods.map((m) => link(m.nameHtml, m.id)) },
        { label: "Events", links: events.map((e) => link(e.nameHtml, e.id)) },
        {
            label: `From ${packageName("std")}`,
            links: [
                ...addedProperties.map((r) => link(r.nameHtml, r.id)),
                ...addedMethods.map((m) => link(m.nameHtml, m.id)),
            ],
        },
    ].filter((g) => g.links.length);

    const count = (n: number) => (n ? n : undefined);
    const outline: OutlineItem[] = [
        ...(example ? [{ id: "sec-example", label: "Example" }] : []),
        { id: "sec-members", label: "Members" },
        { id: "sec-properties", label: "Properties", count: count(properties.length) },
        { id: "sec-methods", label: "Methods", count: count(methods.length) },
        { id: "sec-events", label: "Events", count: count(events.length) },
        ...(added.length
            ? [
                  {
                      id: "sec-added",
                      label: `Added by ${packageName("std")}`,
                      count: addedProperties.length + addedMethods.length,
                  },
              ]
            : []),
        ...(elsewhere.length
            ? [{ id: "sec-elsewhere", label: "Added to other classes", count: elsewhere.length }]
            : []),
        { id: "sec-errors", label: "Errors" },
        ...(types.length ? [{ id: "sec-types", label: "Types" }] : []),
    ]
        .filter((item) => item.id !== "sec-properties" || properties.length)
        .filter((item) => item.id !== "sec-methods" || methods.length)
        .filter((item) => item.id !== "sec-events" || events.length);

    return {
        kind: c.kind === "service" ? "service" : "class",
        titleHtml: spelled(c.names),
        titleText: c.names.typescript,
        since: c.since,
        ledeHtml: firstSentence(c.description),
        bodyHtml: afterFirstSentence(c.description),
        importHtml: spelled(c.import),
        importText: c.import,
        extendsHtml: base?.names ? `<a class="ty" href="${base.href}">${spelled(base.names)}</a>` : null,
        returnedBy: { shown: shownReturned, more: returned.length - shownReturned.length },
        exampleHtml: example,
        glance,
        properties,
        methods,
        events,
        added: added.length
            ? { package: packageName("std"), properties: addedProperties, methods: addedMethods }
            : null,
        addedElsewhere: elsewhere,
        errors: { rows: errors, prose: ERROR_PROSE },
        types,
        crumbs: crumbs(pkg, c.area, { html: spelled(c.names) }),
        outline,
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Types pages

export function typesEntries(): { package: Pkg; area: string }[] {
    const out: { package: Pkg; area: string }[] = [];
    for (const pkg of PKGS) {
        for (const area of reference().namespaces[pkg].areas)
            if (hasTypes(pkg, area.id)) out.push({ package: pkg, area: slug(area.id) });
    }
    return out;
}

export async function typesPage(pkg: string, areaSlug: string): Promise<{ pkg: Pkg; page: TypesPage } | undefined> {
    if (pkg !== "core" && pkg !== "std") return undefined;
    const data = reference();
    const namespace = data.namespaces[pkg];
    const area = namespace.areas.find((a) => slug(a.id) === areaSlug);
    if (!area || !hasTypes(pkg, area.id)) return undefined;
    const sections: TypesSection[] = [];
    for (const name of area.classes) {
        const c = data.classes.get(key(pkg, name))!;
        if (isPage(c)) continue;
        const text = describe(c.description);
        if (c.kind === "record") {
            sections.push({
                id: c.name,
                label: c.names.typescript,
                kind: "record",
                nameHtml: spelled(c.names),
                since: c.since,
                descHtml: text,
                bodyHtml: fieldsTable(pkg, c),
            });
        } else {
            const rows = await Promise.all(c.methods.map((m) => methodRow(pkg, m)));
            sections.push({
                id: c.name,
                label: c.names.typescript,
                kind: "interface",
                nameHtml: spelled(c.names),
                since: c.since,
                descHtml: text,
                bodyHtml: rows
                    .map((r) => `<div class="mrow"><div class="sig">${r.sigHtml}</div><p>${r.summaryHtml}</p></div>`)
                    .join(""),
            });
        }
    }
    for (const name of area.enums) {
        const e = data.enums.get(key(pkg, name))!;
        sections.push({
            id: e.name,
            label: e.names.typescript,
            kind: "enum",
            nameHtml: spelled(e.names),
            since: e.since,
            descHtml: describe(e.description),
            bodyHtml: table(
                `Values of ${e.names.typescript}`,
                [{ head: "Value" }, { head: "Description" }],
                e.values.map((v) => ({ cells: [`<b class="nm">${spelled(v.names)}</b>`, inline(v.description)] })),
            ),
        });
    }
    for (const name of area.callbacks) {
        const cb = data.raw.namespaces.find((n) => n.name === namespace.name)!.callbacks.find((c) => c.name === name)!;
        sections.push({
            id: cb.name,
            label: cb.name,
            kind: "callback",
            nameHtml: `<code>${esc(cb.name)}</code>`,
            since: cb.since,
            descHtml: describe(cb.description),
            bodyHtml: cb.params.length
                ? table(
                      "Parameters",
                      [{ head: "Parameter" }, { head: "Type" }, { head: "Notes" }],
                      cb.params.map((p) => ({
                          cells: [
                              `<b class="nm">${esc(p.name)}</b>`,
                              `<code>${esc(p.type)}</code>`,
                              inline(p.description),
                          ],
                      })),
                  )
                : "",
        });
    }
    const title = areaTitle(pkg, area.id);
    return {
        pkg,
        page: {
            package: packageName(pkg),
            area: title,
            sections,
            crumbs: crumbs(pkg, area.id, { html: "Types" }),
            outline: sections.map((s) => ({ id: s.id, label: s.label })),
        },
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Errors

export function errorsPage(): ErrorsPage {
    const data = reference();
    const by = new Map<string, string[]>();
    for (const pkg of PKGS) {
        const namespace = data.namespaces[pkg];
        for (const c of namespace.classes.filter(isPage)) {
            const href = data.entities.get(key(pkg, c.name))!.href;
            const note = (member: { name: string; names: Property["names"]; raises: string[] }, call: string) => {
                for (const code of member.raises) {
                    const list = by.get(code) ?? [];
                    list.push(
                        `<a href="${href}#${member.name}">${spelled(c.names)}.${spelled(member.names)}${call}</a>`,
                    );
                    by.set(code, list);
                }
            };
            for (const p of c.properties) note(p, "");
            for (const m of c.methods) note(m, "()");
        }
    }
    const rows = [...data.errorCodes].map(([code, meaning]) => ({
        code,
        meaning: inline(meaning),
        ...shown(by.get(code) ?? []),
    }));
    return {
        rows,
        prose: ERROR_PROSE,
        crumbs: [REFERENCE_CRUMB, { html: "Errors" }],
        outline: [
            { id: "codes", label: "Codes", count: rows.length },
            { id: "failures", label: "Failures" },
        ],
    };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Paths and search

let paths: Set<string> | undefined;

/** Every page of the reference: a link from the docs is checked against these. */
export function referencePaths(): Set<string> {
    if (paths) return paths;
    paths = new Set(["/reference", "/reference/errors"]);
    for (const e of classEntries()) paths.add(`/reference/${e.package}/${e.name}`);
    for (const e of typesEntries()) paths.add(`/reference/${e.package}/types/${e.area}`);
    return paths;
}

type Spellings = Record<LanguageId, string>;

/** One record for each class and each member, found by any of its three spellings. */
export function searchRecords(): SearchRecord[] {
    const data = reference();
    const out: SearchRecord[] = [];
    for (const pkg of PKGS) {
        const namespace = data.namespaces[pkg];
        for (const c of namespace.classes) {
            const entity = data.entities.get(key(pkg, c.name))!;
            const trail = `${packageName(pkg)} › ${areaTitle(pkg, c.area)}`;
            const kind = c.kind === "handle" ? "class" : c.kind;
            out.push(record(entity.href, kind, c.names, trail, c.description));
            if (!isPage(c) && c.kind !== "record") continue;
            const memberTrail = `${trail} › ${c.names.typescript}`;
            const qualify = (name: Spellings, call: string): Spellings => ({
                typescript: `${c.names.typescript}.${name.typescript}${call}`,
                python: `${c.names.python}.${name.python}${call}`,
                rust: `${c.names.rust}.${name.rust}${call}`,
            });
            for (const p of c.properties)
                out.push(
                    record(`${entity.href}#${p.name}`, "property", qualify(p.names, ""), memberTrail, p.description),
                );
            if (!isPage(c)) continue;
            for (const m of c.methods)
                out.push(
                    record(`${entity.href}#${m.name}`, "method", qualify(m.names, "()"), memberTrail, m.description),
                );
            for (const s of c.signals) {
                const names = {
                    typescript: s.listener.typescript.event,
                    python: s.listener.python.event,
                    rust: s.listener.rust.method,
                };
                out.push(
                    record(`${entity.href}#event-${s.name}`, "event", qualify(names, ""), memberTrail, s.description),
                );
            }
        }
        for (const e of namespace.enums) {
            const entity = data.entities.get(key(pkg, e.name))!;
            out.push(
                record(entity.href, "enum", e.names, `${packageName(pkg)} › ${areaTitle(pkg, e.area)}`, e.description),
            );
        }
        for (const augment of namespace.augments) {
            const target = data.classes.get(key("core", augment.target));
            const entity = data.entities.get(key("core", augment.target));
            if (!target || !entity) continue;
            const trail = `${packageName(pkg)} › ${areaTitle(pkg, augment.area)} › ${target.names.typescript}`;
            const qualify = (name: Spellings, call: string): Spellings => ({
                typescript: `${target.names.typescript}.${name.typescript}${call}`,
                python: `${target.names.python}.${name.python}${call}`,
                rust: `${target.names.rust}.${name.rust}${call}`,
            });
            for (const p of augment.properties)
                out.push(record(`${entity.href}#${p.name}`, "property", qualify(p.names, ""), trail, p.description));
            for (const m of augment.methods)
                out.push(record(`${entity.href}#${m.name}`, "method", qualify(m.names, "()"), trail, m.description));
        }
    }
    return out;
}

function record(url: string, kind: string, title: Spellings, trail: string, description: string): SearchRecord {
    const words = new Set<string>();
    for (const lang of LANGUAGE_IDS) {
        words.add(title[lang]);
        // `Client.appId` is found by `appId` too.
        for (const part of title[lang].split(".")) words.add(part.replace(/\(\)$/, ""));
    }
    return {
        url,
        kind,
        title,
        trail,
        content: `${[...words].join(" ")} ${description.replace(/\s+/g, " ").trim()}`,
    };
}

export type { OutlineItem, TypesSection };
