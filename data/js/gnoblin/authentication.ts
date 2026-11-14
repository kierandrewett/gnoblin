// `gnoblin.authentication`: watch the session's polkit authentications, and refuse one by policy.
//
// When a program needs administrator rights, polkit asks the session's authentication agent. The compositor draws the
// dialog itself, asks for the password in it, and checks the password with polkit's own helper
// (`polkit-agent-helper-1`); a script is never in that path. It cannot draw the dialog, cannot read what is typed in
// it, and cannot say that a password was right. What a script may do is watch: a shell that listens is shown each
// request for a short window before the dialog opens, and may refuse it (a policy that only ever makes things
// stricter).
//
//     import { AuthenticationListener, defineConfig } from "gnoblin";
//
//     class NoRoot extends AuthenticationListener {
//         onAuthenticationRequest(request) {
//             if (request.identities.includes("unix-user:root") && request.subject !== "pkexec") {
//                 request.decline("only pkexec may ask for root");
//             }
//         }
//     }
//     defineConfig({ setup: (gnoblin) => gnoblin.authentication.addListener(new NoRoot()) });
//
// There is no `allow`, no password, and no way to give an identity: the slot the shell holds accepts only the verdict
// `deny`, and everything else (a listener that did nothing, one that threw, one that ran out its budget, no shell at
// all) leaves the request to the compositor's dialog. Listeners run as queued calls under the same wall-clock budget as
// every other listener.
//
// The manager is a view of `gnoblin.dialogs`: the compositor offers an authentication on the same observation slot as
// its consent and passphrase dialogs, so a shell that listens to both holds one slot.

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
    /** The program asking, as polkit reports it (`pkexec`, a process name), or the empty string. Not an identity to trust. */
    readonly subject: string;
    /** Who may authenticate, as polkit writes them: `unix-user:alice`. */
    readonly identities: readonly string[];
}

/** Watches the session's polkit authentications. Every method is optional. */
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

    constructor(dialogs: DialogManager) {
        this.#dialogs = dialogs;
    }

    /** The listeners, in the order they were added. */
    get listeners(): readonly AuthenticationListener[] {
        return this.#dialogs.authenticationListeners;
    }

    addListener(listener: AuthenticationListener): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.authentication.addListener: expects an AuthenticationListener");
        }
        this.#dialogs.addAuthenticationListener(listener);
    }

    removeListener(listener: AuthenticationListener): void {
        this.#dialogs.removeAuthenticationListener(listener);
    }
}
