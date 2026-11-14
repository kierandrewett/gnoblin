// `gnoblin.dialogs`: policy over the compositor's own consent, passphrase and authentication dialogs.
//
// The compositor draws these dialogs itself; a script never does. A shell that observes them is shown each request
// for a short window before the dialog opens and may decline it. This module owns the listener classes, the request
// types and `DialogManager`, plain TypeScript over two host functions (`DialogHost`) so it runs without a
// compositor. It does not own the polkit view: read `gnoblin/authentication`, which the manager also serves.

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

/** A consent dialog: the compositor asks the user to let a portal request through. `subject` is the interface. */
export interface ConsentRequest extends DialogRequest {
    readonly kind: "consent";
}

/** The compositor's passphrase prompt. `subject` is the app's own prompt text. */
export interface SecretRequest extends DialogRequest {
    readonly kind: "secret";
}

/**
 * Observes the requests of the compositor's dialogs. Every method is optional.
 *
 * A listener can do one thing with a request: refuse it with `request.decline(reason)`. There is no way to allow a
 * request and no way to read or give a secret. That is the compositor's rule: the slot the shell holds accepts only
 * the verdict `deny`, and everything else (a listener that did nothing, one that threw, one that ran out its
 * budget, a shell that is not there) leaves the request to the built-in dialog. A listener runs as a queued call
 * under the same wall-clock budget as every other listener: one that throws is reported once and the next listener
 * still runs, and one that runs past its budget is interrupted.
 *
 * @example
 * class NoScreenshots extends DialogListener {
 *     onConsentRequest(request) {
 *         if (request.subject === "Screenshot" && request.appId !== "org.gnome.Shell") {
 *             request.decline("screenshots are off");
 *         }
 *     }
 * }
 * defineConfig({ setup: (gnoblin) => gnoblin.dialogs.addListener(new NoScreenshots()) });
 */
export class DialogListener {
    /** A consent dialog is about to open. */
    onConsentRequest?(request: ConsentRequest): void;
    /** The passphrase prompt is about to open. */
    onSecretRequest?(request: SecretRequest): void;
}

/** What the manager needs of the shell: the call queue and the way a verdict goes back. */
export interface DialogHost {
    /** Queues `task` to run after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Hands the verdict on `request` to the shell, which sends it to the compositor. */
    verdict(request: number, deny: boolean, reason: string): void;
}

/**
 * One slot event as the shell hands it over. The compositor reuses one record shape for every dialog, so the fields
 * carry different things by `title`: see `classify`.
 */
export interface DialogEvent {
    /** `offered`: a request opened. `finished`: its window ended. `lost`: the slot was lost and every request is void. */
    kind: "offered" | "finished" | "lost";
    /** The compositor's id for the request. */
    request?: number;
    /** When the compositor takes the request back, in milliseconds on its own clock. */
    deadline?: number;
    /** The kind tag: `consent:<interface>`, `secret` or `auth`. */
    title?: string;
    /** The prompt text (passphrase) or the sentence polkit wants shown (authentication). */
    message?: string;
    /** The verified app id (consent, passphrase) or the authentication's details as JSON. */
    subject?: string;
    /** Sent by the compositor with a finished event. Not read here. */
    completion?: string;
}

/**
 * The request a listener is shown. The private fields are why a script cannot decline a request twice with
 * different reasons, nor read the verdict: only the manager reaches them, through the statics.
 */
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

    /** The first call wins. A call after the window has closed does nothing. Throws a `TypeError` for a non-string reason. */
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

    /** Marks the observation window as ended, so a later `decline` is ignored. */
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

/** The name of a listener's class, for the report when one of its calls throws; `listener` when it has none. */
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

    /** `host` queues the listener calls and carries the verdicts back to the compositor. */
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

    /** Adds a listener once; adding it again does nothing. Throws a `TypeError` when `listener` is not an object. */
    addListener(listener: DialogListener): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.dialogs.addListener: expects a DialogListener");
        }
        if (!this.#listeners.includes(listener)) {
            this.#listeners.push(listener);
        }
    }

    /** Removes a listener. A listener that is not there is ignored. */
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

    /** Removes an authentication listener. A listener that is not there is ignored. */
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

    // Each listener gets its own queued call, so one that throws or runs out its budget leaves the others their turn.
    // The settling call is queued last and runs last, because the queue runs in order.
    #offered(event: DialogEvent): void {
        const id = event.request ?? 0;
        const found = classify(event);
        if (found === null) {
            // A request this library does not know how to describe is not refused: it passes.
            this.#host.verdict(id, false, "");
            return;
        }
        const deadline = event.deadline ?? 0;
        const request =
            found.kind === "auth"
                ? this.#observeAuthentication(found, event.message ?? "", deadline)
                : this.#observeDialog(found, event.subject ?? "", deadline);
        this.#held.set(id, request);
        this.#host.schedule("dialogs.settle", () => {
            this.#held.delete(id);
            const outcome = ObservedRequest.outcome(request);
            if (!outcome.closed) {
                this.#host.verdict(id, outcome.deny, outcome.reason);
            }
        });
    }

    /** Queues the authentication listeners' calls for a polkit request and returns the request they are shown. */
    #observeAuthentication(
        found: Extract<Classified, { kind: "auth" }>,
        message: string,
        deadline: number,
    ): ObservedAuthentication {
        const authentication = new ObservedAuthentication(
            found.actionId,
            message,
            found.requester,
            found.identities,
            deadline,
        );
        // Iterate a copy: a listener may remove itself while the calls are being queued.
        for (const listener of [...this.#authListeners]) {
            if (typeof listener.onAuthenticationRequest === "function") {
                this.#host.schedule(`${describe(listener)}.onAuthenticationRequest`, () => {
                    // A listener removed since the call was queued is not called.
                    if (this.#authListeners.includes(listener)) {
                        listener.onAuthenticationRequest?.(authentication);
                    }
                });
            }
        }
        return authentication;
    }

    /** Queues the listeners' calls for a consent or passphrase request and returns the request they are shown. */
    #observeDialog(
        found: Extract<Classified, { kind: "consent" | "secret" }>,
        appId: string,
        deadline: number,
    ): ObservedRequest<"consent"> | ObservedRequest<"secret"> {
        const observed =
            found.kind === "consent"
                ? new ObservedRequest("consent", appId, found.subject, deadline)
                : new ObservedRequest("secret", appId, found.subject, deadline);
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
        return observed;
    }

    // The window ended (answered, refused, timed out): a late `decline` has nothing to refuse.
    #finished(id: number): void {
        const request = this.#held.get(id);
        if (request !== undefined) {
            // The window ended (answered, refused, timed out): a late `decline` has nothing to refuse.
            ObservedRequest.close(request);
        }
    }
}
