import { Collection, Style as NativeStyle } from "gnoblin-std/generated/classes";
import type { QueryTerm } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import type { ComputedStyle } from "gnoblin-std/style/computed";
/** The library's number of each scalar and list property, by the script's name. */
declare const PROPS: {
    readonly opacity: 1;
    readonly cornerRadius: 2;
    readonly background: 3;
    readonly borders: 4;
    readonly shadows: 5;
    readonly filter: 6;
    readonly backdropFilter: 7;
};
export type NativeProp = keyof typeof PROPS;
/** What the library holds of one scope, and what it was last given. */
export interface Scope {
    readonly native: NativeStyle;
    /** The content of the script's style the library was last given, as JSON. */
    given: string;
}
/** The library's style of the scope of `target`: a collection of clients, or one client's node. */
export declare function scopeOf(kind: "collection" | "client", target: number): NativeStyle;
/** A query of `where` as the library's terms: its keys are the library's (`appId` is `app_id`), a list is alternatives. */
export declare function termsOf(query: Record<string, unknown>): Init<QueryTerm>[];
/** The library's root collection of clients. */
export declare function clientsRoot(): Collection;
/** Hands the library the scalars and lists a style holds (`toJSON`), replacing what it had: nothing is written when they did not change. */
export declare function give(scope: Scope, content: Record<string, unknown>): void;
/** The list a scope would hand a script that does not override `name`: what it inherits, as plain data. */
export declare function inherited(scope: Scope, name: "borders" | "shadows" | "filter" | "backdropFilter"): Record<string, unknown>[];
/** A client's resolved style: frozen data, and which layer each property came from. */
export declare function computedOf(node: number): ComputedStyle;
/** What a client's style asks for that nothing draws, one note each. */
export declare function notesOf(node: number): string[];
/** What a client that is gone and was never asked resolves to: nothing set. */
export declare function emptyComputed(): ComputedStyle;
export {};
