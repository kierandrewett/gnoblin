/**
 * Starts this generation's styles from nothing in the library. A reload that stays in one core generation leaves the library's styles
 * and what it drew on the windows, so the generation clears them first (its own styles are made after); a library that starts empty
 * has nothing to clear. The library is then the owner of the styles, and its first pass takes what is left off the windows.
 */
export declare function installStyles(): void;
/** Checks the arrays a script changed in place, gives the library the styles that moved, and says what it did not draw. */
export declare function applyStyles(): void;
