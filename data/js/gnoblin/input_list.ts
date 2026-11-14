// The live lists of `gnoblin.input`: `contexts` and `devices` (owner-spec.md section 10).
//
// Each is a `Collection<T>` in the specification: iterable, with `length`, `at`, `get`, `find`, `filter` and `map`. They
// read what the manager holds each time, so a list a script kept shows later changes. Only `contexts` takes `add` and
// `remove`: a script makes contexts, while devices are the compositor's.

/** A live, read-only list. `read` answers the members now, each time it is asked. */
export class InputList<T extends { readonly id: string }> implements Iterable<T> {
    readonly #read: () => readonly T[];

    constructor(read: () => readonly T[]) {
        this.#read = read;
    }

    /** How many members there are now. */
    get length(): number {
        return this.#read().length;
    }

    /** The member at `index`; a negative index counts from the end. Undefined past either end. */
    at(index: number): T | undefined {
        return this.#read().at(index);
    }

    /** The member whose `id` is `id`. */
    get(id: string): T | undefined {
        return this.#read().find((member) => member.id === id);
    }

    /** The first member `predicate` accepts. */
    find(predicate: (member: T) => boolean): T | undefined {
        return this.#read().find(predicate);
    }

    /** A snapshot array of the members `predicate` accepts. */
    filter(predicate: (member: T) => boolean): T[] {
        return this.#read().filter(predicate);
    }

    /** A snapshot array of `mapper` applied to each member. */
    map<U>(mapper: (member: T) => U): U[] {
        return this.#read().map(mapper);
    }

    /** A snapshot array of the lists `mapper` returns for each member, joined. */
    flatMap<U>(mapper: (member: T) => readonly U[]): U[] {
        return this.#read().flatMap(mapper);
    }

    [Symbol.iterator](): Iterator<T> {
        return this.#read()[Symbol.iterator]();
    }
}

/** A list a script may add members to and remove them from. The owner of the list decides what each does. */
export class MutableInputList<T extends { readonly id: string }> extends InputList<T> {
    readonly #add: (member: T) => T;
    readonly #remove: (member: T) => boolean;

    constructor(read: () => readonly T[], add: (member: T) => T, remove: (member: T) => boolean) {
        super(read);
        this.#add = add;
        this.#remove = remove;
    }

    /** Adds `member` and returns it. */
    add(member: T): T {
        return this.#add(member);
    }

    /** Removes `member`. False when it is not in the list. */
    remove(member: T): boolean {
        return this.#remove(member);
    }
}
