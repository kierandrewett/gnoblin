// Naming helper for the fields of host values.

/** Converts underscores followed by lowercase letters or digits to camelCase. */
export function camel(name: string): string {
    return name.replace(/_([a-z0-9])/g, (_, letter: string) => letter.toUpperCase());
}
