// What a shell's lock screen reads of the compositor's session lock: the state, the lock node, the passwords it sends
// and the answers that come back. The shell host provides it (`gnoblin-core/host/contract`); `gnoblin-std/lock` is the
// stock screen that draws on it. A shell with no lock screen leaves the Rust fallback card in charge
// (`docs/session-lock.md`).

/** The compositor's answer to a password check: `result` is `ok`, or the wait in milliseconds under `rate-limited`. */
export interface LockAnswer {
    /** Identifies the password check that produced this answer. */
    token: number;
    /** Outcome of the check, for example `ok` or `rate-limited`. */
    result: string;
    /** Error detail supplied by the compositor, or an empty string. */
    error: string;
}

/** The functions the shell host provides to the lock screen. A test attaches a fake one. */
export interface LockBackend {
    /** The session lock's state: `not-locked`, `pending` or `locked`. A tree with no lock node is `not-locked`. */
    state(): string;
    /** The lock node, which the screen is mounted under, or `undefined` when the tree has none. */
    node(): number | undefined;
    /** The width and height of the screen the lock screen covers, or `undefined` before the tree has one. */
    screen(): number[] | undefined;
    /** Records whether this screen is up. Returns the host's refusal code, if any. */
    engage(engaged: boolean): string | undefined;
    /** Asks the compositor to check the password. Returns the token the answer comes back under. */
    authenticate(password: string): number | undefined;
    /**
     * Renews the lease the compositor gives the screen. A screen that stops renewing it (hung, or failing every frame)
     * loses the password entry to the compositor's own lock card. The host sends it at most twice a second.
     */
    heartbeat(): void;
    /** Returns and clears answers received since the previous call. */
    takeAnswers(): LockAnswer[];
}
