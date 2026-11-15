// Naming helpers for event names and suggestions for unknown names.

/** Converts underscores followed by lowercase letters or digits to camelCase. */
export function camel(name: string): string {
    return name.replace(/_([a-z0-9])/g, (_, letter: string) => letter.toUpperCase());
}

/** Calculates the number of single-character edits between two names. */
function distance(a: string, b: string): number {
    let previous = Array.from({ length: b.length + 1 }, (_, index) => index);
    for (let row = 1; row <= a.length; row++) {
        const current = [row];
        for (let column = 1; column <= b.length; column++) {
            const cost = a[row - 1] === b[column - 1] ? 0 : 1;
            current.push(Math.min(previous[column] + 1, current[column - 1] + 1, previous[column - 1] + cost));
        }
        previous = current;
    }
    return previous[b.length];
}

/** The maximum edit distance for suggesting a near match. */
const NEAR_DISTANCE = 3;

/** Returns names with the same final segment or within the suggestion distance. */
export function nearMatches(name: string, known: readonly string[]): string[] {
    const last = name.split(".").pop() ?? name;
    return known
        .map((candidate) => ({ candidate, score: distance(candidate, name) }))
        .filter(({ candidate, score }) => candidate.endsWith(`.${last}`) || score <= NEAR_DISTANCE)
        .sort((a, b) => a.score - b.score)
        .slice(0, 5)
        .map(({ candidate }) => candidate);
}
