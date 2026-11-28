// What the reference pages show: the view models the build-time loader (src/lib/server/reference.ts) hands to the page components. Every
// string named `html` is markup the loader wrote: names, types and declarations come from reference.json, written once for each language
// that spells them differently (`data-only`, see html.ts), so a page holds the three languages and styles/reference.css shows one.

export type Html = string;

export interface Crumb {
    html: Html;
    href?: string;
}

export interface NavClass {
    html: Html;
    href: string;
}

export interface NavArea {
    id: string;
    title: string;
    classes: NavClass[];
    /** The area's types page, when it has records, enums, interfaces or callbacks. */
    types: string | null;
}

export interface NavPackage {
    id: "core" | "std";
    name: string;
    areas: NavArea[];
}

export interface ReferenceNav {
    packages: NavPackage[];
}

export interface OutlineItem {
    id: string;
    label: string;
    count?: number;
}

/** A name or a call, with the page that holds it: "Client.fullscreen", "ClientCollection.at()". */
export interface MemberFact {
    html: Html;
    href: string;
}

export interface IndexGroup {
    label: string;
    links: NavClass[];
}

export interface IndexArea {
    id: string;
    title: string;
    count: number;
    groups: IndexGroup[];
}

export interface IndexPackage {
    id: "core" | "std";
    name: string;
    blurb: string;
    /** The import line of one of its classes, for each language. */
    importHtml: Html;
    importText: Record<"typescript" | "python" | "rust", string>;
    areas: IndexArea[];
}

export interface IndexPage {
    packages: IndexPackage[];
    errors: { codes: { code: string }[] };
}

/** A row of a properties table: a property of a class or a field of a record. */
export interface PropertyRow {
    id: string;
    nameHtml: Html;
    typeHtml: Html;
    /** The declaration lines of the member in each language (the Rust table shows these instead of a name and a type). */
    declHtml: Html;
    access: "read/write" | "read-only";
    animatable: boolean;
    /** The first paragraph of the description. */
    descHtml: Html;
    /** The rest of the description: shown when the row is opened by its anchor. */
    moreHtml: Html;
    /** Default, range and unit, each a short line. */
    facts: Html[];
}

export interface EventRow {
    id: string;
    /** The name a listener is given: `"titleChanged"`, `"title_changed"` and `on_title_changed`. */
    nameHtml: Html;
    declHtml: Html;
    /** The arguments the handler receives. */
    argsHtml: Html;
    descHtml: Html;
    moreHtml: Html;
}

export interface MethodRow {
    id: string;
    /** Open when the page is: a constructor is not hidden. */
    open?: boolean;
    /** `apply()`: the name, for the list of members. */
    nameHtml: Html;
    /** The signature each language declares. */
    sigHtml: Html;
    summaryHtml: Html;
    /** What opens under the signature: the rest of the description, parameters, returns, errors and an example. */
    bodyHtml: Html;
}

export interface Group<T> {
    heading: string;
    note?: Html;
    rows: T[];
}

export interface MemberLink {
    html: Html;
    href: string;
}

export interface GlanceGroup {
    label: string;
    links: MemberLink[];
}

/** A member another library adds to a class, listed on the page of the class it extends and on the page of the class it belongs to. */
export interface AddedRow {
    id: string;
    /** `Client.workspace`, `Gnoblin.show()`: the class and the member. */
    memberHtml: Html;
    memberHref: string;
    typeHtml: Html;
    declHtml: Html;
    descHtml: Html;
}

export interface TypeRow {
    id: string;
    nameHtml: Html;
    href: string;
    kind: string;
    /** The values of an enum or the fields of a record, in each language. */
    valuesHtml: Html;
    descHtml: Html;
}

export interface ErrorRow {
    code: string;
    meaning: Html;
    /** The members that raise it, as links; at most a few, and how many more there are. */
    by: Html[];
    more: number;
}

export interface ClassPage {
    kind: "class" | "service";
    /** The class name in each language, for the title. */
    titleHtml: Html;
    titleText: string;
    since: string;
    ledeHtml: Html;
    bodyHtml: Html;
    /** `import { Client } from "gnoblin-core";` in each language. */
    importHtml: Html;
    importText: Record<"typescript" | "python" | "rust", string>;
    extendsHtml: Html | null;
    /** How an object of the class comes to be, in a sentence; null when the schema does not say. */
    createdBy: Html | null;
    ctor: MethodRow | null;
    returnedBy: { shown: MemberLink[]; more: number };
    exampleHtml: Html;
    glance: GlanceGroup[];
    properties: PropertyRow[];
    methods: MethodRow[];
    events: EventRow[];
    /** The events the schema declares: the count of the section (the generic row is not counted). */
    eventCount: number;
    /** What the other library adds to this class, and what this library adds to others. */
    added: { package: string; properties: PropertyRow[]; methods: MethodRow[] } | null;
    addedElsewhere: AddedRow[];
    errors: { rows: ErrorRow[]; prose: Record<"typescript" | "python" | "rust", Html> };
    types: TypeRow[];
    crumbs: Crumb[];
    outline: OutlineItem[];
}

export interface TypesSection {
    id: string;
    /** The name in TypeScript, for the outline. */
    label: string;
    kind: "record" | "interface" | "enum" | "callback";
    nameHtml: Html;
    since: string;
    descHtml: Html;
    /** Records and interfaces: the fields or methods. Enums: the values. Callbacks: the parameters. */
    bodyHtml: Html;
}

export interface TypesPage {
    package: string;
    area: string;
    sections: TypesSection[];
    crumbs: Crumb[];
    outline: OutlineItem[];
}

export interface ErrorsPage {
    rows: ErrorRow[];
    prose: Record<"typescript" | "python" | "rust", Html>;
    crumbs: Crumb[];
    outline: OutlineItem[];
}

/** One entry of the search index: a class or a member, with its name in each language. */
export interface SearchRecord {
    url: string;
    kind: string;
    title: Record<"typescript" | "python" | "rust", string>;
    /** Where it lives: `gnoblin-core › Windows and outputs › Client`. */
    trail: string;
    /** Every spelling and the description: what the query is matched against. */
    content: string;
}
