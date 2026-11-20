/** A live, read-only list. `read` answers the members now, each time it is asked. */
export declare class InputList<T extends {
    readonly id: string;
}> implements Iterable<T> {
    #private;
    constructor(read: () => readonly T[]);
    /** How many members there are now. */
    get length(): number;
    /** The member at `index`; a negative index counts from the end. Undefined past either end. */
    at(index: number): T | undefined;
    /** The member whose `id` is `id`. */
    get(id: string): T | undefined;
    /** The first member `predicate` accepts. */
    find(predicate: (member: T) => boolean): T | undefined;
    /** A snapshot array of the members `predicate` accepts. */
    filter(predicate: (member: T) => boolean): T[];
    /** A snapshot array of `mapper` applied to each member. */
    map<U>(mapper: (member: T) => U): U[];
    /** A snapshot array of the lists `mapper` returns for each member, joined. */
    flatMap<U>(mapper: (member: T) => readonly U[]): U[];
    /** Iterates the current members. */
    [Symbol.iterator](): Iterator<T>;
}
/** A list a script may add members to and remove them from. The owner of the list decides what each does. */
export declare class MutableInputList<T extends {
    readonly id: string;
}> extends InputList<T> {
    #private;
    constructor(read: () => readonly T[], add: (member: T) => T, remove: (member: T) => boolean);
    /** Adds `member` and returns it. */
    add(member: T): T;
    /** Removes `member`. False when it is not in the list. */
    remove(member: T): boolean;
}
