// `gnoblin.authentication`: watch the session's polkit authentications, and refuse one by policy.
//
// The compositor draws the polkit dialog and checks the password itself; a script is never in that path. A script
// sees each request for a short window before the dialog opens and may only decline it. This module owns the
// listener class, the request type and the manager view. It does not own the dialog. Read `gnoblin/dialogs`
// first: the manager is a view of `gnoblin.dialogs`, which holds the one observation slot.

import type { DialogManager, DialogRequest } from "gnoblin/dialogs";

/**
 * A polkit authentication the compositor is about to open its dialog for. It lives for the observation window and no
 * longer: `decline` after the window has closed does nothing. It carries nothing the password check needs.
 */
export interface AuthenticationRequest extends DialogRequest {
    readonly kind: "auth";
    /** The polkit action, for example `org.freedesktop.policykit.exec`. */
    readonly actionId: string;
    /** The sentence polkit wants shown, in the session's language. */
    readonly message: string;
    /**
     * The application id the compositor verified for whoever asks. Always empty: polkit names a process, not an
     * application, so the program asking is `subject`.
     */
    readonly appId: string;
    /**
     * The program asking, as polkit reports it (`pkexec`, a process name), or the empty string. Not an identity to
     * trust.
     */
    readonly subject: string;
    /** Who may authenticate, as polkit writes them: `unix-user:alice`. */
    readonly identities: readonly string[];
}

/**
 * Watches the session's polkit authentications. Every method is optional.
 *
 * A script cannot draw the dialog, read what is typed in it, or say that a password was right. The slot the shell
 * holds accepts only the verdict `deny`: a listener that did nothing, one that threw, one that ran out its budget,
 * and no shell at all all leave the request to the compositor's dialog. There is no `allow`, no password and no way
 * to give an identity. Listeners run as queued calls under the same wall-clock budget as every other listener.
 *
 * @example
 * class NoRoot extends AuthenticationListener {
 *     onAuthenticationRequest(request) {
 *         if (request.identities.includes("unix-user:root") && request.subject !== "pkexec") {
 *             request.decline("only pkexec may ask for root");
 *         }
 *     }
 * }
 * defineConfig({ setup: (gnoblin) => gnoblin.authentication.addListener(new NoRoot()) });
 */
export class AuthenticationListener {
    /**
     * An authentication is about to be asked of the user. `request.decline(reason)` refuses it: polkitd is told it was
     * not authorized and the dialog never opens. Doing nothing leaves it to the dialog.
     */
    onAuthenticationRequest?(request: AuthenticationRequest): void;
}

/** `gnoblin.authentication`: the listeners that watch the session's polkit authentications. */
export class AuthenticationManager {
    readonly #dialogs: DialogManager;

    /** `dialogs` is the manager whose observation slot carries the authentications. */
    constructor(dialogs: DialogManager) {
        this.#dialogs = dialogs;
    }

    /** The listeners, in the order they were added. */
    get listeners(): readonly AuthenticationListener[] {
        return this.#dialogs.authenticationListeners;
    }

    /** Adds a listener. Throws a `TypeError` when `listener` is not an object. */
    addListener(listener: AuthenticationListener): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.authentication.addListener: expects an AuthenticationListener");
        }
        this.#dialogs.addAuthenticationListener(listener);
    }

    /** Removes a listener that was added. A listener that is not there is ignored. */
    removeListener(listener: AuthenticationListener): void {
        this.#dialogs.removeAuthenticationListener(listener);
    }
}
