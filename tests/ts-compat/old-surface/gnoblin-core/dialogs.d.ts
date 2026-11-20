import type { AuthenticationListener } from "gnoblin-core/authentication";
/**
 * What a request is: a consent dialog for a portal request, the compositor's passphrase prompt, or a polkit
 * authentication.
 */
export type DialogKind = "consent" | "secret" | "auth";
/**
 * A request the compositor is about to open a dialog for. It lives for the observation window and no longer:
 * `decline` after the window has closed does nothing.
 */
export interface DialogRequest {
    /** The kind of dialog request. */
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
    /** Identifies this as a portal consent request. */
    readonly kind: "consent";
}
/** The compositor's passphrase prompt. `subject` is the app's own prompt text. */
export interface SecretRequest extends DialogRequest {
    /** Identifies this as a passphrase request. */
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
 * gnoblin.dialogs.addListener(new NoScreenshots());
 */
export declare class DialogListener {
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
    /**
     * `offered`: a request opened. `finished`: its window ended. `lost`: the slot was lost and every request is void.
     */
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
/** `gnoblin.dialogs`: the listeners, and the requests they have been shown and not yet settled. */
export declare class DialogManager {
    #private;
    /** `host` queues the listener calls and carries the verdicts back to the compositor. */
    constructor(host: DialogHost);
    /** The listeners, in the order they were added. */
    get listeners(): readonly DialogListener[];
    /** The authentication listeners (`gnoblin.authentication`), in the order they were added. */
    get authenticationListeners(): readonly AuthenticationListener[];
    /** Whether the shell should hold the observation slot: it is held while any listener could use it. */
    get wanted(): boolean;
    /** Adds a listener once; adding it again does nothing. Throws a `TypeError` when `listener` is not an object. */
    addListener(listener: DialogListener): void;
    /** Removes a listener. A listener that is not there is ignored. */
    removeListener(listener: DialogListener): void;
    /** Adds a listener of polkit authentications. The shell calls this for `gnoblin.authentication`. */
    addAuthenticationListener(listener: AuthenticationListener): void;
    /** Removes an authentication listener. A listener that is not there is ignored. */
    removeAuthenticationListener(listener: AuthenticationListener): void;
    /** What the compositor sent since the last frame. The shell calls this; a script does not. */
    deliver(events: readonly DialogEvent[]): void;
}
