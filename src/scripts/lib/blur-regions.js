import Meta from 'gi://Meta';

// Bounds belong to the socket peer's own layer surfaces. They do not enable
// blur: window rules remain the authority for whether an effect exists.
export class BlurRegions {
    constructor() { this.clients = new Map(); }

    update(client, record) {
        if (typeof record.namespace !== 'string' || record.namespace.length > 128 ||
            !Array.isArray(record.screen) || record.screen.length !== 2 ||
            !record.screen.every(n => Number.isFinite(n) && Math.abs(n) <= 65536) ||
            (record.region !== null && (!Array.isArray(record.region) || record.region.length !== 4 ||
                !record.region.every(n => Number.isFinite(n) && Math.abs(n) <= 65536) ||
                record.region[2] < 0 || record.region[3] < 0)))
            throw new Error('Invalid blur region');
        let state = this.clients.get(client);
        if (!state) {
            const pid = client.connection.get_socket().get_credentials().get_unix_pid();
            state = {pid, regions: new Map()};
            this.clients.set(client, state);
        }
        const key = JSON.stringify([record.namespace, record.screen]);
        if (!state.regions.has(key) && state.regions.size >= 64)
            throw new Error('Too many blur regions');
        if (record.region === null) state.regions.delete(key);
        else state.regions.set(key, record.region);
        for (const actor of global.get_window_actors()) this.apply(actor);
    }

    apply(actor) {
        const window = actor.meta_window;
        const namespace = Meta.gnoblin_layer_namespace(window);
        if (namespace === null || window.get_monitor() < 0) return;
        const screen = global.display.get_monitor_geometry(window.get_monitor());
        const key = JSON.stringify([namespace, [screen.x, screen.y]]);
        let region = null;
        for (const state of this.clients.values()) {
            if (state.pid === window.get_pid() && state.regions.has(key)) {
                region = state.regions.get(key);
                break;
            }
        }
        const previous = actor._gnoblinBlurRegion;
        actor._gnoblinBlurRegion = region;
        const effect = actor.get_effect('gnoblin-window-blur');
        if (region) effect?.set_region?.(...region);
        else if (previous) effect?.clear_region?.();
    }

    close(client) {
        if (!this.clients.delete(client)) return;
        for (const actor of global.get_window_actors()) this.apply(actor);
    }
}
