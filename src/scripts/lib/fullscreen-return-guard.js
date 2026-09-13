// Consume an outside click that races the search layer while it is open over
// a fullscreen client. The press and matching release are one safety action,
// not input for the application underneath the shell chrome.
export class FullscreenReturnGuard {
    constructor({ buttonPress, buttonRelease, pick, dismiss, set }) {
        this.buttonPress = buttonPress;
        this.buttonRelease = buttonRelease;
        this.pick = pick;
        this.dismiss = dismiss;
        this.set = set;
        this.armed = false;
        this.blockedButton = 0;
    }

    arm() {
        this.armed = true;
        this.blockedButton = 0;
        this.set(true);
    }

    disarm() {
        this.armed = false;
        this.blockedButton = 0;
        this.set(false);
    }

    handle(event) {
        const type = event.type();
        if (type !== this.buttonPress && type !== this.buttonRelease) return false;
        const button = event.get_button?.() || 0;

        if (type === this.buttonRelease && button === this.blockedButton) {
            this.blockedButton = 0;
            return true;
        }

        if (!this.armed || type !== this.buttonPress || button !== 1) return false;

        const window = this.pick(event);
        if (!window?.is_fullscreen?.()) return false;

        this.armed = false;
        this.blockedButton = button;
        this.dismiss();
        return true;
    }
}
