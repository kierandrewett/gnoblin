/** A record a caller gives in part: the fields it leaves out are zero. */
export type Init<T> = {
    [K in keyof T]?: T[K] extends (infer E)[] ? E[] : T[K] extends object ? Init<T[K]> : T[K];
};
/** Calls the table function `name` with `args`, for the generation that is running. */
export declare function call(name: string, ...args: unknown[]): any;
/** The names of the table functions this build of the runtime knows, in table order. */
export declare function functions(): string[];
