// The shape of reference.json (written by `cargo xtask gen-sdks --target reference`; crates/gnoblin-sdkgen/src/reference.rs says what
// each key holds). The site reads these names and declarations as they are and spells nothing itself.

export const LANGUAGE_IDS = ["typescript", "python", "rust"] as const;
export type LanguageId = (typeof LANGUAGE_IDS)[number];
/** A value spelled in each language. */
export type Spelled = Record<LanguageId, string>;
/** The lines each language's SDK file declares. */
export type Declared = Record<LanguageId, string[]>;

export interface TypeRef {
    kind: "class" | "enum" | "callback";
    name: string;
}

export interface TypeSpelling extends Spelled {
    schema: string;
    refs: TypeRef[];
}

export interface Range {
    min: number;
    max: number;
}

export type DefaultValue = boolean | number | string | DefaultValue[];

export interface Example {
    typescript: string | null;
    python: string | null;
    rust: string | null;
}

export interface Property {
    name: string;
    names: Spelled;
    since: string;
    description: string;
    type: TypeSpelling;
    readonly: boolean;
    optional: boolean;
    always: boolean;
    animated: boolean;
    secret: boolean;
    default: DefaultValue | null;
    range: Range | null;
    unit: string | null;
    raises: string[];
    example: Example | null;
    declarations: Declared;
}

/** A parameter or a field of a signal. */
export interface Field {
    name: string;
    names: Spelled;
    since: string;
    description: string;
    type: TypeSpelling;
    optional: boolean;
    secret: boolean;
    default: DefaultValue | null;
    range: Range | null;
    unit: string | null;
}

export interface Method {
    name: string;
    names: Spelled;
    since: string;
    description: string;
    static: boolean;
    required: boolean;
    secret: boolean;
    params: Field[];
    returns: { type: TypeSpelling; optional: boolean } | null;
    operation: boolean;
    raises: string[];
    example: Example | null;
    declarations: Declared;
}

export interface Signal {
    name: string;
    since: string;
    description: string;
    secret: boolean;
    payload: Field[];
    listener: {
        typescript: { method: string; event: string };
        python: { method: string; event: string };
        rust: { method: string };
    };
    example: Example | null;
    declarations: Declared;
}

export interface Variant {
    name: string;
    tag: string;
    since: string;
    description: string;
    properties: {
        name: string;
        field: string | null;
        description: string;
        type: string;
        default: DefaultValue | null;
        range: Range | null;
        unit: string | null;
    }[];
}

export type ClassKind = "handle" | "service" | "record" | "interface";

export interface RawClass {
    name: string;
    names: Spelled;
    area: string;
    kind: ClassKind;
    base: string | null;
    since: string;
    description: string;
    secret: boolean;
    import: Spelled;
    example: Example | null;
    properties: Property[];
    methods: Method[];
    signals: Signal[];
    variants: Variant[];
}

export interface RawAugment {
    name: string;
    target: string;
    area: string;
    since: string;
    properties: Property[];
    methods: Method[];
}

export interface EnumValue {
    name: string;
    id: number;
    names: Spelled;
    description: string;
    declarations: Declared;
}

export interface RawEnum {
    name: string;
    names: Spelled;
    area: string;
    since: string;
    description: string;
    values: EnumValue[];
}

export interface RawCallback {
    name: string;
    area: string;
    since: string;
    description: string;
    params: { name: string; type: string; description: string }[];
    returns: string | null;
    optional: boolean;
}

export interface RawArea {
    id: string;
    title: string | null;
    file: string;
    classes: string[];
    enums: string[];
    callbacks: string[];
    augments: string[];
}

export interface RawNamespace {
    name: string;
    version: string;
    fingerprint: string;
    package: {
        typescript: { package: string; module: string };
        python: { module: string };
        rust: { crate: string; module: string };
    };
    counts: Record<string, number>;
    areas: RawArea[];
    classes: RawClass[];
    enums: RawEnum[];
    callbacks: RawCallback[];
    augments: RawAugment[];
}

export interface RawReference {
    format: number;
    catalog: string;
    languages: { id: LanguageId; label: string; result: { type: string; import: string } | null }[];
    namespaces: RawNamespace[];
}
