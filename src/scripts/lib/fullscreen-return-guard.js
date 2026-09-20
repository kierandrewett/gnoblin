// Keep fullscreen pointer input behind revealed shell chrome. The second left
// click starts dismissal, and the client resumes only after every companion
// has completed its exit animation.
export class FullscreenReturnGuard {
    constructor({ buttonPress, buttonRelease, pointerEvents, pick, dismiss, set }) {
        this.buttonPress = buttonPress;
        this.buttonRelease = buttonRelease;
        this.pointerEvents = pointerEvents;
        this.pick = pick;
        this.dismiss = dismiss;
        this.set = set;
        this.armed = false;
        this.dismissing = false;
        this.blockedButtons = new Set();
    }

    update(search, requests) {
        const revealed =
            search?.revealCompanions === true &&
            ["bingux-search", "bingux-search-chrome"].includes(search.surface) &&
            requests.some((request) => request.surface === search.surface);
        if (revealed) this.arm();
        else this.disarm();
    }

    arm() {
        if (this.armed) return;
        this.armed = true;
        this.set(true);
    }

    disarm() {
        if (!this.armed) return;
        this.armed = false;
        this.set(false);
    }

    handle(event) {
        const type = event.type();
        if (!this.pointerEvents.includes(type)) return false;
        const isButton = type === this.buttonPress || type === this.buttonRelease;
        const button = isButton ? event.get_button?.() || 0 : 0;

        if (type === this.buttonRelease && this.blockedButtons.delete(button)) {
            return true;
        }

        if (!this.armed) return false;

        const window = this.pick(event);
        if (!window?.is_fullscreen?.()) return false;

        if (type === this.buttonPress && button > 0) this.blockedButtons.add(button);
        if (type === this.buttonPress && button === 1 && !this.dismissing) {
            this.dismissing = true;
            this.dismiss(() => {
                this.dismissing = false;
                this.disarm();
            });
        }
        return true;
    }
}
