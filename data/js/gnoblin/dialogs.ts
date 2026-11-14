// `gnoblin.dialogs`: policy over the compositor's own consent, passphrase and authentication dialogs.
//
// The compositor draws these dialogs itself (a script never does) and a shell that observes them is shown each
// request for a short window before the dialog opens. A listener can do one thing with a request: refuse it,
// `request.decline(reason)`. There is no way to allow a request and no way to read or give a secret, and that is
// the compositor's rule, not this module's: the slot the shell holds accepts only the verdict `deny`, and
// everything else (a listener that did nothing, one that threw, one that ran out its budget, a shell that is not
// there) leaves the request to the built-in dialog.
//
//     import { DialogListener, defineConfig } from "gnoblin";
//
//     class NoScreenshots extends DialogListener {
//         onConsentRequest(request) {
//             if (request.subject === "Screenshot" && request.appId !== "org.gnome.Shell") {
//                 request.decline("screenshots are off");
//             }
//         }
//     }
//     defineConfig({ setup: (gnoblin) => gnoblin.dialogs.addListener(new NoScreenshots()) });
//
// A polkit authentication is the third kind of request. It is watched through `gnoblin.authentication`
// (`AuthenticationListener`, in `gnoblin/authentication`), which this manager serves on the same observation slot:
// the compositor draws the password dialog and checks the password with polkit's helper, and a script can only
// refuse the request.
//
// The manager is plain TypeScript over two functions of the host (`DialogHost`), so it runs without a compositor.
// A listener runs as a queued call under the same wall-clock budget as every other listener: one that throws is
// reported once and the next listener still runs, and one that runs past its budget is interrupted.

import type { AuthenticationListener, AuthenticationRequest } from "gnoblin/authentication";

/** What a request is: a consent dialog for a portal request, the compositor's passphrase prompt, or a polkit authentication. */
export type DialogKind = "consent" | "secret" | "auth";

/**
 * A request the compositor is about to open a dialog for. It lives for the observation window and no longer:
 * `decline` after the window has closed does nothing.
 */
export interface DialogRequest {
    readonly kind: DialogKind;
    /** The app id the compositor verified for whoever asks. Empty when the request names none. */
    readonly appId: string;
    /**
     * What is asked about. For a consent request, the portal interface (`Screenshot`, `ScreenCast`, ...). For a
     * passphrase request, the app's own prompt text, which is the app's words and not to be trusted as identity. For
     * an authentication, the program asking.
     */
    readonly subject: string;
    /** When the compositor takes the request back, in milliseconds on its own clock. */
    readonly deadline: number;
    /**
     * Refuses the request by policy. The requester is told it was refused and the dialog never opens. `reason` is
     * for the log. There is no counterpart that allows a request.
     */
    decline(reason?: string): void;
}

export interface ConsentRequest extends DialogRequest {
    readonly kind: "consent";
}

export interface SecretRequest extends DialogRequest {
    readonly kind: "secret";
}

/** Observes the requests of the compositor's dialogs. Every method is optional. */
export class DialogListener {
    onConsentRequest?(request: ConsentRequest): void;
    onSecretRequest?(request: SecretRequest): void;
}

/** What the manager needs of the shell: the call queue and the way a verdict goes back. */
export interface DialogHost {
    /** Queues `task` to run after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Hands the verdict on `request` to the shell, which sends it to the compositor. */
    verdict(request: number, deny: boolean, reason: string): void;
}

/** One slot event as the shell hands it over. */
export interface DialogEvent {
    kind: "offered" | "finished" | "lost";
    request?: number;
    deadline?: number;
    title?: string;
    message?: string;
    subject?: string;
    completion?: string;
}

class ObservedRequest<K extends DialogKind> implements DialogRequest {
    readonly kind: K;
    readonly appId: string;
    readonly subject: string;
    readonly deadline: number;
    #declined = false;
    #reason = "";
    #closed = false;

    constructor(kind: K, appId: string, subject: string, deadline: number) {
        this.kind = kind;
        this.appId = appId;
        this.subject = subject;
        this.deadline = deadline;
    }

    decline(reason: string = "policy"): void {
        if (typeof reason !== "string") {
            throw new TypeError("request.decline: the reason must be a string");
        }
        if (this.#closed || this.#declined) {
            return;
        }
        this.#declined = true;
        this.#reason = reason;
    }

    /** What the listeners decided. Only the manager holds the class, so a script cannot read or set this. */
    static outcome(request: ObservedRequest<DialogKind>): { deny: boolean; reason: string; closed: boolean } {
        return { deny: request.#declined, reason: request.#reason, closed: request.#closed };
    }

    static close(request: ObservedRequest<DialogKind>): void {
        request.#closed = true;
    }
}

/** A polkit authentication, as an `AuthenticationListener` sees it. */
class ObservedAuthentication extends ObservedRequest<"auth"> implements AuthenticationRequest {
    readonly actionId: string;
    readonly message: string;
    readonly identities: readonly string[];

    constructor(actionId: string, message: string, requester: string, identities: readonly string[], deadline: number) {
        // polkit names a process and never an application, so the verified application id is always empty.
        super("auth", "", requester, deadline);
        this.actionId = actionId;
        this.message = message;
        this.identities = Object.freeze([...identities]);
    }
}

/** A request the manager holds, by its kind, so a listener's method gets the request type it declares. */
type Observed = ObservedRequest<"consent"> | ObservedRequest<"secret"> | ObservedAuthentication;

/** What the compositor told of an offer, read by its kind. */
type Classified =
    | { kind: "consent" | "secret"; subject: string }
    | { kind: "auth"; actionId: string; requester: string; identities: string[] };

/** The part of an authentication the compositor shows an observer: `{ actionId, requester, identities }` as JSON. */
function readAuthentication(text: string): { actionId: string; requester: string; identities: string[] } {
    let parsed: unknown = null;
    try {
        parsed = JSON.parse(text);
    } catch {
        // A payload that is not JSON leaves every field empty: the request is still observed, and still refusable.
    }
    const object =
        parsed !== null && typeof parsed === "object"
            ? (parsed as { [key: string]: unknown })
            : ({} as { [key: string]: unknown });
    const identities = Array.isArray(object.identities) ? object.identities : [];
    return {
        actionId: typeof object.actionId === "string" ? object.actionId : "",
        requester: typeof object.requester === "string" ? object.requester : "",
        identities: identities.filter((identity): identity is string => typeof identity === "string"),
    };
}

/**
 * The kind and subject of an offer, from the title the compositor gave it: `consent:<interface>`, `secret` or `auth`.
 * An authentication's own details are in its subject, as JSON.
 */
function classify(event: DialogEvent): Classified | null {
    const title = event.title ?? "";
    if (title.startsWith("consent:")) {
        return { kind: "consent", subject: title.slice("consent:".length) };
    }
    if (title === "secret") {
        return { kind: "secret", subject: event.message ?? "" };
    }
    if (title === "auth") {
        return { kind: "auth", ...readAuthentication(event.subject ?? "") };
    }
    return null;
}

function describe(listener: object): string {
    const name: string | undefined = listener.constructor?.name;
    return name !== undefined && name !== "" ? name : "listener";
}

/** Runs the method of `listener` that takes a consent or passphrase request. */
function notify(listener: DialogListener, request: ObservedRequest<"consent"> | ObservedRequest<"secret">): void {
    if (request.kind === "consent") {
        listener.onConsentRequest?.(request);
    } else {
        listener.onSecretRequest?.(request);
    }
}

/** `gnoblin.dialogs`: the listeners, and the requests they have been shown and not yet settled. */
export class DialogManager {
    readonly #host: DialogHost;
    readonly #listeners: DialogListener[] = [];
    readonly #authListeners: AuthenticationListener[] = [];
    readonly #held = new Map<number, Observed>();

    constructor(host: DialogHost) {
        this.#host = host;
    }

    /** The listeners, in the order they were added. */
    get listeners(): readonly DialogListener[] {
        return [...this.#listeners];
    }

    /** The authentication listeners (`gnoblin.authentication`), in the order they were added. */
    get authenticationListeners(): readonly AuthenticationListener[] {
        return [...this.#authListeners];
    }

    /** Whether the shell should hold the observation slot: it is held while any listener could use it. */
    get wanted(): boolean {
        return this.#listeners.length > 0 || this.#authListeners.length > 0;
    }

    addListener(listener: DialogListener): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.dialogs.addListener: expects a DialogListener");
        }
        if (!this.#listeners.includes(listener)) {
            this.#listeners.push(listener);
        }
    }

    removeListener(listener: DialogListener): void {
        const index = this.#listeners.indexOf(listener);
        if (index >= 0) {
            this.#listeners.splice(index, 1);
        }
    }

    /** Adds a listener of polkit authentications. The shell calls this for `gnoblin.authentication`. */
    addAuthenticationListener(listener: AuthenticationListener): void {
        if (!this.#authListeners.includes(listener)) {
            this.#authListeners.push(listener);
        }
    }

    removeAuthenticationListener(listener: AuthenticationListener): void {
        const index = this.#authListeners.indexOf(listener);
        if (index >= 0) {
            this.#authListeners.splice(index, 1);
        }
    }

    /** What the compositor sent since the last frame. The shell calls this; a script does not. */
    deliver(events: readonly DialogEvent[]): void {
        for (const event of events) {
            if (event.kind === "offered") {
                this.#offered(event);
            } else if (event.kind === "finished") {
                this.#finished(event.request ?? 0);
            } else if (event.kind === "lost") {
                for (const request of this.#held.values()) {
                    ObservedRequest.close(request);
                }
                this.#held.clear();
            }
        }
    }

    #offered(event: DialogEvent): void {
        const id = event.request ?? 0;
        const found = classify(event);
        if (found === null) {
            // A request this library does not know how to describe is not refused: it passes.
            this.#host.verdict(id, false, "");
            return;
        }
        const deadline = event.deadline ?? 0;
        let request: Observed;
        // One call per listener, so one that throws or runs out its budget leaves the others their turn, then one
        // that settles the request: it runs last because the queue runs in order.
        if (found.kind === "auth") {
            const authentication = new ObservedAuthentication(
                found.actionId,
                event.message ?? "",
                found.requester,
                found.identities,
                deadline,
            );
            request = authentication;
            for (const listener of [...this.#authListeners]) {
                if (typeof listener.onAuthenticationRequest === "function") {
                    this.#host.schedule(`${describe(listener)}.onAuthenticationRequest`, () => {
                        if (this.#authListeners.includes(listener)) {
                            listener.onAuthenticationRequest?.(authentication);
                        }
                    });
                }
            }
        } else {
            const appId = event.subject ?? "";
            const observed =
                found.kind === "consent"
                    ? new ObservedRequest("consent", appId, found.subject, deadline)
                    : new ObservedRequest("secret", appId, found.subject, deadline);
            request = observed;
            const method = found.kind === "consent" ? "onConsentRequest" : "onSecretRequest";
            for (const listener of [...this.#listeners]) {
                if (typeof listener[method] === "function") {
                    this.#host.schedule(`${describe(listener)}.${method}`, () => {
                        if (this.#listeners.includes(listener)) {
                            notify(listener, observed);
                        }
                    });
                }
            }
        }
        this.#held.set(id, request);
        this.#host.schedule("dialogs.settle", () => {
            this.#held.delete(id);
            const outcome = ObservedRequest.outcome(request);
            if (!outcome.closed) {
                this.#host.verdict(id, outcome.deny, outcome.reason);
            }
        });
    }

    #finished(id: number): void {
        const request = this.#held.get(id);
        if (request !== undefined) {
            // The window ended (answered, refused, timed out): a late `decline` has nothing to refuse.
            ObservedRequest.close(request);
        }
    }
}
