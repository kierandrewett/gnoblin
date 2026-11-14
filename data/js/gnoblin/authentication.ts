// `gnoblin.authentication`: be the session's authentication agent.
//
// When a program needs administrator rights, polkit asks an authentication agent, the compositor offers the request to
// whoever holds the `auth.polkit` slot, and the holder shows a prompt and answers. A shell that sets a provider holds
// the slot while it is set, so the requests come to it instead of to the services role's built-in agent; with none set,
// the slot is released and the default provider serves them again.
//
//     import { AuthenticationProvider, defineConfig } from "gnoblin";
//
//     class Prompt extends AuthenticationProvider {
//         onRequest(request) {
//             // Show request.title and request.message, then, when the user has proved who they are:
//             request.allow("unix-user:kieran");
//             // or, when they say no:
//             // request.decline("denied");
//         }
//     }
//     defineConfig({ setup: (gnoblin) => gnoblin.authentication.setProvider(new Prompt()) });
//
// What the compositor reads of an answer on this slot: a non-empty identity is consent, the identity the user
// authenticated as; a decline is a refusal. Nothing here verifies who the user is: the provider is trusted to have
// done that before it calls `allow`, which is why only a shell holding the slot can answer. A request that nobody
// settles before its deadline ends as timed out.
//
// The manager is plain TypeScript over two functions of the host (`AuthenticationHost`), so it runs without a
// compositor. A provider's method runs as a queued call under the same wall-clock budget as every other listener.

/** How a provider can decline: the user is not there to answer (`locked`), said no (`denied`) or the prompt cannot be shown. */
export type AuthenticationDecline = "locked" | "denied" | "unavailable";

/** An authentication request a provider holds. It is open until it is settled, ends or the shell loses its connection. */
export interface AuthenticationRequest {
    /** The compositor's number for the request. */
    readonly id: number;
    /** The heading the requester chose, for the prompt. */
    readonly title: string;
    /** The prompt's body text, for example the action polkit asks about. */
    readonly message: string;
    /** What the request is about, as the offering agent chose it. The compositor does not interpret it. */
    readonly subject: string;
    /** When the compositor takes the request back, on its own clock in milliseconds. */
    readonly deadline: number;
    /** True until the request is settled or has ended. */
    readonly open: boolean;
    /** How it ended: `answered`, `ended`, `refused`, `timed-out` or `lost`; null while it is open. */
    readonly completion: string | null;
    /** The user authenticated as `identity`, a non-empty name such as `unix-user:kieran`. */
    allow(identity: string): void;
    /** The request is refused. `reason` defaults to `"denied"`. */
    decline(reason?: AuthenticationDecline): void;
}

/** Answers the session's authentication requests. Every method is optional. */
export class AuthenticationProvider {
    /** A request was offered. Settle it now or later; a request nobody settles ends at its deadline. */
    onRequest?(request: AuthenticationRequest): void;
    /** A request ended: someone settled it, it timed out, or the requester withdrew it. */
    onFinished?(request: AuthenticationRequest): void;
}

/** What the manager needs of the shell: the call queue and the way a settled request goes back. */
export interface AuthenticationHost {
    schedule(site: string, task: () => void): void;
}

/** One slot event as the shell hands it over. */
export interface AuthenticationEvent {
    kind: "offered" | "finished" | "lost";
    request?: number;
    deadline?: number;
    title?: string;
    message?: string;
    subject?: string;
    completion?: string;
}

/** How a request was settled, for the shell to send. */
export interface AuthenticationReply {
    request: number;
    /** `allow` answers with `identity`; `decline` refuses with `reason`. */
    kind: "allow" | "decline";
    identity: string;
    reason: AuthenticationDecline;
}

const DECLINES: readonly string[] = ["locked", "denied", "unavailable"];

/** The longest identity an answer may carry. */
export const MAX_IDENTITY_BYTES = 256;

/** The length of `text` in UTF-8 bytes: the sandbox has no `TextEncoder`. */
function utf8Length(text: string): number {
    let bytes = 0;
    for (const symbol of text) {
        const code = symbol.codePointAt(0) ?? 0;
        bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    }
    return bytes;
}

class HeldRequest implements AuthenticationRequest {
    readonly id: number;
    readonly title: string;
    readonly message: string;
    readonly subject: string;
    readonly deadline: number;
    #completion: string | null = null;
    readonly #settle: (reply: AuthenticationReply) => void;
    #settled = false;

    constructor(event: AuthenticationEvent, settle: (reply: AuthenticationReply) => void) {
        this.id = event.request ?? 0;
        this.title = event.title ?? "";
        this.message = event.message ?? "";
        this.subject = event.subject ?? "";
        this.deadline = event.deadline ?? 0;
        this.#settle = settle;
    }

    get open(): boolean {
        return this.#completion === null && !this.#settled;
    }

    get completion(): string | null {
        return this.#completion;
    }

    allow(identity: string): void {
        if (typeof identity !== "string" || identity.length === 0) {
            throw new TypeError("request.allow: the identity must be a non-empty string");
        }
        if (utf8Length(identity) > MAX_IDENTITY_BYTES) {
            throw new RangeError(`request.allow: the identity is at most ${MAX_IDENTITY_BYTES} bytes`);
        }
        if (!this.open) {
            return;
        }
        this.#settled = true;
        this.#settle({ request: this.id, kind: "allow", identity, reason: "denied" });
    }

    decline(reason: AuthenticationDecline = "denied"): void {
        if (!DECLINES.includes(reason)) {
            throw new TypeError(`request.decline: the reason is one of ${DECLINES.join(", ")}`);
        }
        if (!this.open) {
            return;
        }
        this.#settled = true;
        this.#settle({ request: this.id, kind: "decline", identity: "", reason });
    }

    /** The manager's side: the request ended. */
    static end(request: HeldRequest, completion: string): void {
        if (request.#completion === null) {
            request.#completion = completion;
        }
    }
}

function describe(provider: object): string {
    const name: string | undefined = provider.constructor?.name;
    return name !== undefined && name !== "" ? name : "provider";
}

/** `gnoblin.authentication`: the provider, and the requests it has been offered and not yet seen end. */
export class AuthenticationManager {
    readonly #host: AuthenticationHost;
    readonly #held = new Map<number, HeldRequest>();
    #replies: AuthenticationReply[] = [];
    #provider: AuthenticationProvider | null = null;

    constructor(host: AuthenticationHost) {
        this.#host = host;
    }

    /** The provider that answers requests, or null when the built-in agent does. */
    get provider(): AuthenticationProvider | null {
        return this.#provider;
    }

    /** Whether the shell should hold the `auth.polkit` slot. */
    get wanted(): boolean {
        return this.#provider !== null;
    }

    /** The requests the provider has been offered that have not ended. */
    get requests(): readonly AuthenticationRequest[] {
        return [...this.#held.values()];
    }

    /**
     * Makes `provider` the session's authentication agent, replacing the one there was. The shell claims the slot at
     * its next frame, so requests are offered to the new provider from then on.
     */
    setProvider(provider: AuthenticationProvider): void {
        if (provider === null || typeof provider !== "object") {
            throw new TypeError("gnoblin.authentication.setProvider: expects an AuthenticationProvider");
        }
        this.#provider = provider;
    }

    /** Gives the slot back: the services role's agent answers from the next frame. */
    clearProvider(): void {
        this.#provider = null;
    }

    /** What the compositor sent since the last frame. The shell calls this; a script does not. */
    deliver(events: readonly AuthenticationEvent[]): void {
        for (const event of events) {
            if (event.kind === "offered") {
                this.#offered(event);
            } else if (event.kind === "finished") {
                this.#finished(event.request ?? 0, event.completion ?? "ended");
            } else {
                for (const [id, request] of [...this.#held]) {
                    this.#finish(id, request, "lost");
                }
            }
        }
    }

    /** The answers to send, each once. The shell calls this; a script does not. */
    takeReplies(): AuthenticationReply[] {
        const taken = this.#replies;
        this.#replies = [];
        return taken;
    }

    #offered(event: AuthenticationEvent): void {
        const provider = this.#provider;
        const id = event.request ?? 0;
        if (provider === null) {
            // The slot is still held for a frame after the provider went: the request is not left to time out.
            this.#replies.push({ request: id, kind: "decline", identity: "", reason: "unavailable" });
            return;
        }
        const request = new HeldRequest(event, (reply) => this.#replies.push(reply));
        this.#held.set(id, request);
        if (typeof provider.onRequest === "function") {
            this.#host.schedule(`${describe(provider)}.onRequest`, () => {
                if (request.open) {
                    provider.onRequest?.(request);
                }
            });
        }
    }

    #finished(id: number, completion: string): void {
        const request = this.#held.get(id);
        if (request !== undefined) {
            this.#finish(id, request, completion);
        }
    }

    #finish(id: number, request: HeldRequest, completion: string): void {
        this.#held.delete(id);
        HeldRequest.end(request, completion);
        const provider = this.#provider;
        if (provider !== null && typeof provider.onFinished === "function") {
            this.#host.schedule(`${describe(provider)}.onFinished`, () => provider.onFinished?.(request));
        }
    }
}
