// Pure rule validation and frame geometry; no texture readback or polling.
export const defaults = Object.freeze({
    radius: 0, smoothing: 0, mode: 'auto', padding: [0, 0, 0, 0],
    'border-width': 0, 'border-color': '#808080ff',
    // Maximized windows still have a visible work-area boundary. Keep their
    // corner mask unless a rule explicitly opts out.
    'keep-maximized': true, 'keep-fullscreen': false, 'keep-tiled': false,
    'skip-libadwaita': true, 'skip-libhandy': false, shadow: false,
    'keep-shadow': false,
    'shadow-animation': {duration: 0, easing: 'ease-out-cubic'},
});

export function validate(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value))
        throw new Error('corners must be a table');
    const number = (v, min, max) => typeof v === 'number' && Number.isFinite(v) && v >= min && v <= max;
    const color = v => typeof v === 'string' && /^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(v);
    for (const [key, v] of Object.entries(value)) {
        if (!Object.hasOwn(defaults, key)) throw new Error(`unknown corners setting: ${key}`);
        let valid;
        if (key === 'radius') valid = number(v, 0, 200);
        else if (key === 'smoothing') valid = number(v, 0, 1);
        else if (key === 'mode') valid = ['auto', 'force', 'off'].includes(v);
        else if (key === 'padding') valid = Array.isArray(v) && v.length === 4 && v.every(n => number(n, -128, 128));
        else if (key === 'border-width') valid = number(v, -40, 40);
        else if (key === 'border-color') valid = color(v);
        else if (key === 'shadow-animation') {
            valid = v && typeof v === 'object' && !Array.isArray(v) && Object.entries(v).every(([k,n]) =>
                k === 'duration' ? number(n,0,2000) : k === 'easing' && ['linear','ease-out-cubic','ease-out-quad','ease-in-out-cubic'].includes(n));
        } else if (key === 'shadow') {
            const layer = item => item && typeof item === 'object' && !Array.isArray(item) && Object.entries(item).every(([k, n]) =>
                k === 'color' ? color(n) : k === 'opacity' ? number(n, 0, 1) :
                    k === 'blur' ? number(n, 0, 100) : ['x', 'y', 'spread'].includes(k) && number(n, -100, 100));
            valid = v === false || (Array.isArray(v) ? v.length > 0 && v.length <= 4 && v.every(layer) : layer(v));
        } else valid = typeof v === 'boolean';
        if (!valid) {
            const expected = {
                radius: 'a number from 0 to 200',
                smoothing: 'a number from 0 to 1',
                mode: '"auto", "force", or "off"',
                padding: 'four numbers from -128 to 128',
                'border-width': 'a number from -40 to 40',
                'border-color': 'a #RRGGBB or #RRGGBBAA color',
                'shadow-animation': 'a duration from 0 to 2000 and a supported easing',
                shadow: 'false, a shadow table, or a list of shadow tables',
            }[key] ?? 'a boolean';
            throw new Error(`invalid corners.${key}: expected ${expected}; got ${JSON.stringify(v)}`);
        }
    }
}

export function merge(previous, next) {
    const result = {...previous, ...next};
    if (next['shadow-animation']) result['shadow-animation'] = {...previous['shadow-animation'], ...next['shadow-animation']};
    if (next.shadow && previous.shadow && !Array.isArray(next.shadow) && !Array.isArray(previous.shadow)) result.shadow = {...previous.shadow, ...next.shadow};
    return result;
}

export function geometry(frame, buffer, width, height, config) {
    if (![frame.x, frame.y, frame.width, frame.height, buffer.x, buffer.y, buffer.width, buffer.height, width, height].every(Number.isFinite)
        || buffer.width <= 0 || buffer.height <= 0 || width <= 0 || height <= 0) return null;
    const sx = width / buffer.width, sy = height / buffer.height;
    const [top, right, bottom, left] = config.padding;
    const bounds = [Math.max(0, (frame.x - buffer.x + left) * sx), Math.max(0, (frame.y - buffer.y + top) * sy),
        Math.min(width, (frame.x - buffer.x + frame.width - right) * sx),
        Math.min(height, (frame.y - buffer.y + frame.height - bottom) * sy)];
    const limit = Math.min(bounds[2] - bounds[0], bounds[3] - bounds[1]) / 2;
    if (limit <= 1) return null;
    const exponent = 2 + config.smoothing * 10;
    // Match Reborn's expanding superellipse, clamped on both axes.
    const radius = Math.min(config.radius * Math.min(sx, sy) * exponent / 2, limit);
    return {bounds, radius, exponent, scale: Math.min(sx, sy), width, height};
}

export function enabled(config, state) {
    return config.radius > 0 && config.mode !== 'off' && state.normal &&
        (!state.fullscreen || config['keep-fullscreen']) &&
        (!state.maximized || config['keep-maximized']) &&
        (!state.tiled || config['keep-tiled']) &&
        !(config.mode === 'auto' && ((state.adwaita && config['skip-libadwaita']) || (state.handy && config['skip-libhandy'])));
}

// Border policy is independent of clipping and toolkit detection.
export const borderDefaults = Object.freeze({
    'inner-width': 0, 'inner-color': '#808080ff',
    'outer-width': 0, 'outer-color': '#00000080',
    radius: 0, smoothing: 0, padding: [0, 0, 0, 0],
    // Borders are an explicit visual rule. Keep the stroke around maximized
    // and tiled windows, but never decorate a true fullscreen surface unless
    // a rule explicitly opts in.
    'keep-maximized': true, 'keep-fullscreen': false, 'keep-tiled': true,
});
export function validateBorders(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value))
        throw new Error('borders must be a table');
    for (const [key, v] of Object.entries(value)) {
        if (!Object.hasOwn(borderDefaults, key)) throw new Error(`unknown borders setting: ${key}`);
        if (key.endsWith('-width')) {
            if (typeof v !== 'number' || !Number.isFinite(v) || v < 0 || v > 40)
                throw new Error(`invalid borders.${key}`);
        } else if (key.endsWith('-color')) validate({'border-color': v});
        else validate({[key]: v});
    }
}
export function bordersEnabled(config, state) {
    return (config['inner-width'] > 0 || config['outer-width'] > 0) && state.normal &&
        (!state.fullscreen || config['keep-fullscreen']) &&
        (!state.maximized || config['keep-maximized']) &&
        (!state.tiled || config['keep-tiled']);
}

// A shadow ramp is not an edge. Require a sharp alpha step and a stable
// plateau behind it; uncertain content retains the client-provided frame.
export function detectEdge(alpha) {
    const limit = Math.min(128, Math.floor(alpha.length / 4));
    if (limit < 4) return null;
    const reference = alpha[Math.floor(alpha.length / 2)];
    if (reference < 64) return null;
    const stable = i => alpha.slice(i, i + 4).every(a => Math.abs(a-reference) <= 8);
    if (stable(0)) return 0;
    for (let i = 1; i < limit; i++) {
        if (alpha[i] - alpha[i-1] >= 32 && stable(i) &&
            alpha.slice(0, i).every(a => a < reference * .8)) return i;
    }
    return null;
}
