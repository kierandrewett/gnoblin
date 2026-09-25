export function normalizeTouchpadPath(points) {
    const scale = Math.max(...points.map(({ x, y }) => Math.max(Math.abs(x), Math.abs(y))));
    if (scale < 1e-6) return null;
    return points.map(({ x, y }) => ({ x: x / scale, y: y / scale }));
}

function resamplePath(points, count = 16) {
    const lengths = [0];
    for (let i = 1; i < points.length; i++) {
        const dx = points[i].x - points[i - 1].x;
        const dy = points[i].y - points[i - 1].y;
        lengths.push(lengths[i - 1] + Math.hypot(dx, dy));
    }
    const total = lengths.at(-1);
    if (total < 1e-6) return null;
    const result = [];
    let segment = 1;
    for (let i = 0; i < count; i++) {
        const target = (total * i) / (count - 1);
        while (segment < lengths.length - 1 && lengths[segment] < target) segment++;
        const start = lengths[segment - 1];
        const span = lengths[segment] - start;
        const amount = span === 0 ? 0 : (target - start) / span;
        result.push({
            x: points[segment - 1].x + (points[segment].x - points[segment - 1].x) * amount,
            y: points[segment - 1].y + (points[segment].y - points[segment - 1].y) * amount,
        });
    }
    return result;
}

export function touchpadGesturePathMatches(template, points, tolerance = 0.22) {
    if (!template || !points || points.length < 2) return false;
    const expected = resamplePath(normalizeTouchpadPath(template) ?? []);
    const actual = resamplePath(normalizeTouchpadPath(points) ?? []);
    if (!expected || !actual) return false;
    const error = Math.sqrt(
        expected.reduce((sum, point, index) => {
            const dx = point.x - actual[index].x;
            const dy = point.y - actual[index].y;
            return sum + dx * dx + dy * dy;
        }, 0) / expected.length,
    );
    return error <= tolerance;
}

/** Pure gesture recognition core. Shell actions and context are supplied by the caller. */
export class TouchpadGestureRouter {
    constructor({ getContexts, runGesture }) {
        this._getContexts = getContexts;
        this._runGesture = runGesture;
        this._gestures = null;
        this._gesture = null;
    }

    configure(gestures) {
        this._gestures = gestures;
        this._gesture = null;
    }

    destroy() {
        this._gesture = null;
        this._gestures = null;
    }

    handle(payload) {
        const { gesture: kind, phase, fingers } = payload;
        if (phase === "begin") {
            const contexts = this._getContexts();
            const candidates = (this._gestures ?? []).filter(
                (item) =>
                    item.gesture === kind &&
                    item.fingers === fingers &&
                    (item.action === undefined || !item.action.endsWith(".progress")) &&
                    (item.command !== undefined || item.action !== undefined) &&
                    (item.when === "any" || contexts.includes(item.when)),
            );
            this._gesture = {
                kind,
                candidates,
                path: [{ x: 0, y: 0 }],
                x: 0,
                y: 0,
                distance: 0,
                recognized: null,
                claimed: candidates.length > 0,
                initialScale: payload.scale ?? 1,
            };
            return this._gesture.claimed;
        }

        if (!this._gesture || this._gesture.kind !== kind) return false;

        if (kind === "swipe") {
            const dx = payload.dx ?? 0;
            const dy = payload.dy ?? 0;
            this._gesture.x += dx;
            this._gesture.y += dy;
            this._gesture.distance += Math.hypot(dx, dy);
            if (dx !== 0 || dy !== 0) this._gesture.path.push({ x: this._gesture.x, y: this._gesture.y });
            if (phase === "end")
                this._gesture.recognized =
                    this._gesture.candidates.find(
                        (item) =>
                            this._gesture.distance >= item.threshold &&
                            touchpadGesturePathMatches(item.path, this._gesture.path, item.tolerance),
                    ) ?? null;
        } else if (kind === "pinch") {
            const delta = (payload.scale ?? 1) - this._gesture.initialScale;
            if (phase === "end" && Math.abs(delta) >= 0.001) {
                const direction = delta > 0 ? "out" : "in";
                this._gesture.recognized =
                    this._gesture.candidates.find(
                        (item) => item.direction === direction && Math.abs(delta) >= item.threshold,
                    ) ?? null;
            }
        }

        if (phase === "end") {
            const claimed = this._gesture.claimed;
            if (claimed && this._gesture.recognized) this._runGesture(this._gesture.recognized);
            this._gesture = null;
            return claimed;
        }
        if (phase === "cancel") {
            const claimed = this._gesture.claimed;
            this._gesture = null;
            return claimed;
        }
        return this._gesture.claimed;
    }
}
