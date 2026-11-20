import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { Scope } from "gnoblin-std/style/native";
import { ClientStyle } from "gnoblin-std/style/style";
import type { ComputedStyle } from "gnoblin-std/style/computed";
import type { Style } from "gnoblin-std/style/style";
/** The style a collection of clients carries, and where it ranks among the selections. */
export interface ScopeStyle {
    /** The scope's mutable style values. */
    style: ClientStyle;
    /** The order the scope's style was first asked for in. */
    order: number;
    /** The library's scope of the collection. */
    scope: Scope;
}
/** What the script's side keeps of a client's style. */
export interface ClientStyleState {
    /** The client's own overrides, made the first time a script reads `client.style`. */
    style: ClientStyle | null;
    /** The library's scope of the client, made when the client is first asked about. */
    scope: Scope | null;
    /** The effective style values last computed for the client, kept after it is gone. */
    computed: ComputedStyle | null;
}
/** Lazily allocates the per-client style bookkeeping. */
export declare function styleState(client: Client): ClientStyleState;
/** The scope of `state`, made when first asked for. */
export declare function scopeStyle(state: CollectionState): ScopeStyle;
/** Creates and binds the client-owned style override layer. */
export declare function clientStyle(client: Client): ClientStyle;
/** Hands the library what every style holds, when any changed. */
export declare function pushStyles(): void;
/** Resolves the effective style, retaining the last result after the client is gone. */
export declare function computedStyleOf(client: Client): ComputedStyle;
/** Reports style properties the renderer skipped for a client: what the library says it did not draw, and what the compositor did not. */
export declare function styleWarningsOf(client: Client): readonly string[];
/** Every style a script can have changed an array of in place. */
export declare function everyStyle(): Generator<Style>;
