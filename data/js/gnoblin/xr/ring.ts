// `arrangeRing`: where a room puts its panels, on a ring around the head. Plain arithmetic in millimetres that keeps
// the Lua golden numbers of `xr.lua`.

/** The ring's numbers, in millimetres and radians. `SpatialLayout` states them in metres. */
export interface XRRingParams {
    /** The distance from the viewer to the centre of a panel. */
    radius: number;
    /** The height of a panel's centre above the room's floor. */
    eye: number;
    /** One window pixel is this many millimetres. */
    mmPerPx: number;
    /** The focused panel is this much larger ... */
    focusScale: number;
    /** ... and this much closer to the viewer. */
    focusPull: number;
    /** The angle between neighbouring panels. */
    gap: number;
    /** The most the panels reach to either side of the front: panels shrink to fit. */
    halfArc: number;
    /** Panels on the ring; the rest stack behind it. */
    maxPanels: number;
    /** Each tier of stacked panels is this much further back. */
    stackStep: number;
}

/** Default ring dimensions in millimetres and radians. */
export const RING_DEFAULTS: Readonly<XRRingParams> = Object.freeze({
    radius: 1600,
    eye: 1400,
    mmPerPx: 0.75,
    focusScale: 1.15,
    focusPull: 100,
    gap: (3 * Math.PI) / 180,
    halfArc: (150 * Math.PI) / 180,
    maxPanels: 12,
    stackStep: 250,
});

/** What one panel needs of the ring: its pixel size. `id` is anything comparable. */
export interface RingView<I> {
    readonly id: I;
    readonly width: number;
    readonly height: number;
}

/** One panel's ring position in millimetres, its scale and its stack tier. */
export interface RingPose {
    x: number;
    y: number;
    z: number;
    /** Rotation about each axis in radians; only the turn about y is used. */
    rotation: [number, number, number];
    /** Millimetres per pixel for the panel. */
    scale: number;
    focused: boolean;
    /** 0 on the ring and the tier (1, 2, ...) of a stacked panel. */
    behind: number;
}

/** Ring poses in input order and their far-to-near rendering order. */
export interface RingResult {
    /** One pose per view, in the order of the views. */
    poses: RingPose[];
    /** The view indices from the farthest to the nearest: the order the room must list the Views in. */
    order: number[];
}

/**
 * Places panels around a ring from their pixel sizes and the focused id. The focused panel is at the front, with
 * neighbours in creation order on either side (the right side holds the extra one), wrapping around; without a focus,
 * the first panel is at the front.
 */
export function arrangeRing<I>(
    views: readonly RingView<I>[],
    focus: I | null,
    params: Readonly<XRRingParams> = RING_DEFAULTS,
): RingResult {
    const n = views.length;
    const poses: RingPose[] = new Array<RingPose>(n);
    const order: number[] = [];
    if (n === 0) {
        return { poses, order };
    }
    const p = params;
    const focusAt = focus === null ? -1 : views.findIndex((view) => view.id === focus);
    const hasFocus = focusAt >= 0;
    const m = Math.min(n, p.maxPanels);
    const seq = views.map((_, at) => at);
    if (hasFocus && focusAt >= m) {
        [seq[m - 1], seq[focusAt]] = [seq[focusAt], seq[m - 1]];
    }
    let centreSlot = 0;
    if (hasFocus) {
        for (let k = 0; k < m; k++) {
            if (seq[k] === focusAt) centreSlot = k;
        }
    }
    const wrap = (k: number): number => ((k % m) + m) % m;
    const right: number[] = [];
    const left: number[] = [];
    const nr = Math.floor(m / 2);
    for (let j = 1; j <= nr; j++) right.push(seq[wrap(centreSlot + j)]);
    for (let j = 1; j <= m - 1 - nr; j++) left.push(seq[wrap(centreSlot - j)]);
    const centre = seq[centreSlot];
    const span = (i: number, focused: boolean): number =>
        ((focused ? p.focusScale : 1) * Math.max(views[i].width, 0) * p.mmPerPx) / p.radius;
    const aCentre = span(centre, hasFocus);
    // Both sides must fit in halfArc, gaps included; the panels shrink together until they do.
    let shrink = 1;
    for (const side of [right, left]) {
        let need = aCentre / 2;
        for (const i of side) need += span(i, false);
        const allowed = p.halfArc - side.length * p.gap;
        if (need > allowed) shrink = Math.min(shrink, allowed / need);
    }
    const phi = new Map<number, number>([[centre, 0]]);
    for (const [sign, side] of [
        [1, right],
        [-1, left],
    ] as const) {
        let edge = (shrink * aCentre) / 2;
        for (const i of side) {
            const half = (shrink * span(i, false)) / 2;
            const at = edge + p.gap + half;
            phi.set(i, sign * at);
            edge = at + half;
        }
    }
    const put = (i: number, angle: number, radius: number, focused: boolean, tier: number): void => {
        poses[i] = {
            x: radius * Math.sin(angle),
            y: p.eye,
            z: -radius * Math.cos(angle),
            rotation: [0, 0 - angle, 0],
            scale: shrink * (focused ? p.focusScale : 1) * p.mmPerPx,
            focused,
            behind: tier,
        };
    };
    for (let k = 0; k < m; k++) {
        const i = seq[k];
        const focused = hasFocus && i === centre;
        put(i, phi.get(i) ?? 0, focused ? p.radius - p.focusPull : p.radius, focused, 0);
    }
    // Extras sit behind the ring panel at the same angle, a tier further back for every full turn of the ring.
    const tiers: number[][] = [];
    for (let j = 1; j <= n - m; j++) {
        const tier = Math.floor((j - 1) / m) + 1;
        const i = seq[m + j - 1];
        put(i, phi.get(seq[wrap(j - 1)]) ?? 0, p.radius + p.stackStep * tier, false, tier);
        (tiers[tier] ??= []).push(i);
    }
    for (let tier = tiers.length - 1; tier >= 1; tier--) {
        order.push(...(tiers[tier] ?? []));
    }
    const ring: number[] = [];
    for (let k = 0; k < m; k++) {
        if (!(hasFocus && seq[k] === centre)) ring.push(seq[k]);
    }
    ring.sort((a, b) => a - b);
    order.push(...ring);
    if (hasFocus) {
        order.push(centre);
    }
    return { poses, order };
}
