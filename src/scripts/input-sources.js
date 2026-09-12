import Gio from "gi://Gio";
import GLib from "gi://GLib";
import * as Keyboard from "resource:///org/gnome/shell/ui/status/keyboard.js";

// Compatibility service for an already-running Gnoblin Shell predating the
// built-in org.gnoblin.Shell input-source methods. The rebuilt component owns
// the canonical API; this small service makes the same state available without
// replacing the Wayland compositor mid-session.
const NAME = "org.gnoblin.InputSources";
const PATH = "/org/gnoblin/InputSources";
const IFACE = `<node><interface name="${NAME}">
  <method name="ListInputSources"><arg type="a(ssss)" direction="out" name="sources"/></method>
  <method name="GetCurrentInputSource">
    <arg type="s" direction="out" name="type"/><arg type="s" direction="out" name="id"/>
    <arg type="s" direction="out" name="shortName"/><arg type="s" direction="out" name="displayName"/>
  </method>
  <method name="SetInputSource"><arg type="s" direction="in" name="type"/><arg type="s" direction="in" name="id"/></method>
  <signal name="InputSourceChanged">
    <arg type="s" name="type"/><arg type="s" name="id"/>
    <arg type="s" name="shortName"/><arg type="s" name="displayName"/>
  </signal>
  <signal name="InputSourcesChanged"><arg type="b" name="changed"/></signal>
</interface></node>`;

function record(source) {
    return [source?.type ?? "", source?.id ?? "", source?.shortName ?? "", source?.displayName ?? ""];
}

class InputSources {
    constructor() {
        this.manager = Keyboard.getInputSourceManager();
        this.impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this.impl.export(Gio.DBus.session, PATH);
        this.nameId = Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, null, null, () =>
            console.warn("gnoblin-input-sources: lost bus name"),
        );
        this.manager.connectObject(
            "current-source-changed",
            () => this.emitCurrent(),
            "sources-changed",
            () => this.emitSources(),
            this,
        );
    }

    ListInputSources() {
        return Object.values(this.manager.inputSources).map(record);
    }

    GetCurrentInputSource() {
        return record(this.manager.currentSource);
    }

    SetInputSource(type, id) {
        if (typeof type !== "string" || typeof id !== "string")
            throw new Error("input source type and id must be strings");
        const source = Object.values(this.manager.inputSources).find(
            (candidate) => candidate.type === type && candidate.id === id,
        );
        if (!source) throw new Error(`unknown input source: ${type}/${id}`);
        source.activate(true);
    }

    emitCurrent() {
        this.impl.emit_signal("InputSourceChanged", new GLib.Variant("(ssss)", record(this.manager.currentSource)));
    }

    emitSources() {
        this.impl.emit_signal("InputSourcesChanged", new GLib.Variant("(b)", [true]));
        this.emitCurrent();
    }

    destroy() {
        this.manager.disconnectObject(this);
        this.impl.unexport();
        Gio.bus_unown_name(this.nameId);
    }
}

export default function enable(api) {
    const inputSources = new InputSources();
    api._disposers.push(() => inputSources.destroy());
}
