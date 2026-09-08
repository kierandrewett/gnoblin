// Small, asynchronous state and command transport between independent UI
// processes. Each name has one owner. A disconnected owner loses its state
// and any temporary stacking request; no UI call waits for another process.
export class UiSessions {
    constructor(send, scene) {
        this.send = send;
        this.scene = scene;
        this.owners = new Map();
        this.watchers = new Set();
    }

    command(client, record) {
        if (record.action === 'watch') {
            this.watchers.add(client);
            for (const [name, owner] of this.owners)
                this.send(client, {event: 'ui-state', name, state: owner.state});
            return;
        }
        if (typeof record.name !== 'string' || !/^[a-z][a-z0-9-]{0,63}$/.test(record.name))
            throw new Error('Invalid UI session name');
        const owner = this.owners.get(record.name);
        if (record.action === 'state') {
            if (owner && owner.client !== client) throw new Error('UI session already owned');
            if (!record.state || typeof record.state !== 'object' || Array.isArray(record.state))
                throw new Error('Invalid UI state');
            this.owners.set(record.name, {client, state: record.state});
            this.broadcast(record.name, record.state);
            this.reveal();
        } else if (record.action === 'command') {
            if (record.name === 'search' && ['open', 'toggle'].includes(record.command?.action))
                this.scene.cancelDismiss?.();
            if (owner) this.send(owner.client, {event: 'ui-command', name: record.name, command: record.command});
        } else throw new Error('Invalid UI session action');
    }

    broadcast(name, state) {
        for (const client of this.watchers) this.send(client, {event: 'ui-state', name, state});
    }

    reveal() {
        const requests = [...this.owners.values()].map(owner => owner.state)
            .filter(state => (state.visible || state.revealCompanions === true) && typeof state.surface === 'string' && Array.isArray(state.companions))
            .map(state => ({surface: state.surface,
                companions: state.companions.filter(name => typeof name === 'string').slice(0, 16),
                ...(state.companionsAbove === true ? {companionsAbove: true} : {})}));
        const key = JSON.stringify(requests);
        if (key === this.revealKey) return;
        this.revealKey = key;
        this.scene.update(requests);
    }

    close(client) {
        this.watchers.delete(client);
        for (const [name, owner] of this.owners) {
            if (owner.client !== client) continue;
            this.owners.delete(name);
            this.broadcast(name, null);
        }
        this.reveal();
    }
}
