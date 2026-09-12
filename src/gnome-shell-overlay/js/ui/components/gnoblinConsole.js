import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import System from 'system';

import * as Main from '../main.js';
import {ConsoleEvaluator, isInspectable, preview} from './gnoblinConsoleEvaluator.js';

const HISTORY_KEY = 'looking-glass-history';
const MAX_ROWS = 200;
const VERTICAL = Clutter.Orientation.VERTICAL;

function textLabel(text, styleClass = '', selectable = false) {
    const label = new St.Label({text, style_class: styleClass, x_expand: true});
    label.clutter_text.set_line_wrap(true);
    label.clutter_text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR);
    label.clutter_text.set_ellipsize(Pango.EllipsizeMode.NONE);
    label.clutter_text.set_selectable(selectable);
    return label;
}

function button(label, action, styleClass = 'gnoblin-console-button') {
    const actor = new St.Button({label, style_class: styleClass, can_focus: true});
    actor.connect('clicked', action);
    return actor;
}

export const DeveloperConsole = GObject.registerClass(
class DeveloperConsole extends St.Widget {
    _init() {
        super._init({name: 'gnoblin-developer-console', visible: false, reactive: true});
        this._open = false;
        this._destroyed = false;
        this._expanded = false;
        this._pending = 0;
        this._rows = new Set();
        this._inspectStack = [];
        this._history = global.settings.get_strv(HISTORY_KEY).slice(-MAX_ROWS);
        this._historyIndex = this._history.length;
        this._historyDraft = '';

        this._panel = new St.BoxLayout({
            style_class: 'gnoblin-console', orientation: VERTICAL, reactive: true,
        });
        this.add_child(this._panel);
        const toolbar = new St.BoxLayout({style_class: 'gnoblin-console-toolbar'});
        toolbar.add_child(new St.Label({text: 'GNOBLIN', style_class: 'gnoblin-console-title'}));
        toolbar.add_child(new St.Label({text: 'JavaScript console', style_class: 'gnoblin-console-context'}));
        toolbar.add_child(new St.Widget({x_expand: true}));
        toolbar.add_child(button('Clear', () => this.clear()));
        toolbar.add_child(button('Reset context', () => this.reset()));
        this._expandButton = button('Expand', () => {
            this._expanded = !this._expanded;
            this._expandButton.label = this._expanded ? 'Reduce' : 'Expand';
            this._resize();
        });
        toolbar.add_child(this._expandButton);
        toolbar.add_child(button('Close  Esc', () => this.close()));
        this._panel.add_child(toolbar);

        this._body = new St.BoxLayout({y_expand: true, style_class: 'gnoblin-console-body'});
        this._transcript = new St.BoxLayout({orientation: VERTICAL, x_expand: true});
        this._scroll = new St.ScrollView({
            child: this._transcript, x_expand: true, y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
        });
        this._body.add_child(this._scroll);
        this._inspector = new St.BoxLayout({
            orientation: VERTICAL, visible: false, style_class: 'gnoblin-console-inspector',
        });
        const inspectorHeader = new St.BoxLayout({style_class: 'gnoblin-console-inspector-header'});
        inspectorHeader.add_child(button('Back', () => {
            this._inspectStack.pop();
            if (this._inspectStack.length)
                this._showProperties(this._inspectStack.at(-1));
            else
                this._inspector.hide();
        }));
        inspectorHeader.add_child(textLabel('Inspector', 'gnoblin-console-context'));
        inspectorHeader.add_child(button('Close', () => {
            this._inspectStack = [];
            this._inspector.hide();
        }));
        this._inspector.add_child(inspectorHeader);
        this._properties = new St.BoxLayout({orientation: VERTICAL});
        this._inspector.add_child(new St.ScrollView({
            child: this._properties, y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
        }));
        this._body.add_child(this._inspector);
        this._panel.add_child(this._body);

        this._completions = new St.BoxLayout({style_class: 'gnoblin-console-completions', visible: false});
        this._panel.add_child(this._completions);
        const input = new St.BoxLayout({style_class: 'gnoblin-console-input'});
        input.add_child(new St.Label({text: '>', style_class: 'gnoblin-console-prompt'}));
        this._entry = new St.Entry({
            style_class: 'gnoblin-console-entry', x_expand: true, can_focus: true,
            hint_text: 'Evaluate JavaScript in the compositor',
        });
        this._entry.clutter_text.set_single_line_mode(false);
        this._entry.clutter_text.set_activatable(false);
        this._entry.clutter_text.set_line_wrap(true);
        this._entry.clutter_text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR);
        this._entry.clutter_text.connect('key-press-event', (_text, event) => this._inputKey(event));
        this._entry.clutter_text.connect('text-changed', () => this._hideCompletions());
        input.add_child(this._entry);
        input.add_child(button('Run', () => this._submit(), 'gnoblin-console-run'));
        this._panel.add_child(input);
        const footer = new St.BoxLayout({style_class: 'gnoblin-console-footer'});
        footer.add_child(textLabel('Enter run   Shift+Enter newline   Tab complete   Up/Down history', 'gnoblin-console-hint'));
        this._status = new St.Label({text: 'Compositor context', style_class: 'gnoblin-console-context'});
        footer.add_child(this._status);
        this._panel.add_child(footer);

        this._newEvaluator();
        this._welcome();
        Main.layoutManager.addTopChrome(this, {
            affectsInputRegion: true, affectsStruts: false, trackFullscreen: false,
        });
        Main.layoutManager.connectObject('monitors-changed', () => {
            if (this._open)
                this._resize();
        }, this);
        Main.sessionMode.connectObject('updated', () => {
            if (!this._allowed())
                this.close(true);
        }, this);
        this.connect('destroy', () => {
            this._destroyed = true;
            this.close(true);
            if (this._scrollIdle)
                GLib.source_remove(this._scrollIdle);
            this._inspectStack = [];
            this._rows.clear();
            this._evaluator.clear();
        });
    }

    _newEvaluator() {
        const evaluator = new ConsoleEvaluator({
            global, Main, stage: global.stage, Clutter, Gio, GLib, GObject,
            Meta, Shell, St, Pango, System,
            windows: () => global.get_window_actors().map(actor => actor.meta_window),
        }, {
            onLog: (level, values) => {
                if (!this._destroyed && this._evaluator === evaluator)
                    this._appendValues(level, values);
            },
            onInspect: value => this.inspectObject(value),
            onClear: () => {
                if (!this._destroyed)
                    this._clearTranscript();
            },
        });
        this._evaluator = evaluator;
    }

    _welcome() {
        const row = new St.BoxLayout({orientation: VERTICAL, style_class: 'gnoblin-console-welcome'});
        row.add_child(textLabel('Inspect the running compositor.', 'gnoblin-console-welcome-title'));
        row.add_child(textLabel('Main, global, Meta, St and windows() are available. Use await, $_ or r(index).', 'gnoblin-console-hint'));
        const examples = new St.BoxLayout({style_class: 'gnoblin-console-examples'});
        for (const source of ['windows()', 'Main.layoutManager.monitors', 'inspect(global.stage)'])
            examples.add_child(button(source, () => {
                this._entry.set_text(source);
                this._entry.grab_key_focus();
            }));
        row.add_child(examples);
        this._transcript.add_child(row);
    }

    _allowed() {
        return global.session_mode === 'gnoblin' && Main.sessionMode.isPrimary && !Main.sessionMode.isLocked;
    }

    get isOpen() { return this._open; }
    getIt() { return this._evaluator.lastValue; }
    getResult(id) { return this._evaluator.result(id); }

    toggle() {
        if (this._open)
            this.close();
        else
            this.open();
    }

    open() {
        if (this._open || !this._allowed() || this._destroyed)
            return false;
        this._resize();
        this._panel.remove_all_transitions();
        Main.uiGroup.set_child_above_sibling(this, null);
        this.show();
        const grab = Main.pushModal(this, {actionMode: Shell.ActionMode.LOOKING_GLASS});
        if (grab.get_seat_state() !== Clutter.GrabState.ALL) {
            Main.popModal(grab);
            this.hide();
            return false;
        }
        this._grab = grab;
        this._open = true;
        this._entry.grab_key_focus();
        // pushModal() focuses the modal actor itself. Set the text entry as the
        // final focus target so the first keystroke is ready for JavaScript.
        global.stage.set_key_focus(this._entry);
        this._panel.translation_y = -this.height;
        this._panel.ease({translation_y: 0, duration: this._duration(), mode: Clutter.AnimationMode.EASE_OUT_QUAD});
        return true;
    }

    _duration() {
        return St.Settings.get().enable_animations ? 140 : 0;
    }

    close(immediate = false) {
        if (!this._open && !this.visible)
            return;
        this._open = false;
        this._hideCompletions();
        this._panel.remove_all_transitions();
        if (this._grab) {
            Main.popModal(this._grab);
            this._grab = null;
        }
        if (immediate || !this._duration()) {
            this.hide();
            return;
        }
        this._panel.ease({
            translation_y: -this.height, duration: this._duration(), mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onComplete: () => {
                if (!this._open)
                    this.hide();
            },
        });
    }

    _resize() {
        const monitor = Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        const height = Math.round(monitor.height * (this._expanded ? .9 : .5));
        this.set_position(monitor.x, monitor.y);
        this.set_size(monitor.width, height);
        this.set_clip(0, 0, monitor.width, height + 24);
        this._panel.set_size(monitor.width, height);
        this._inspector.width = Math.min(420, Math.floor(monitor.width * .36));
    }

    vfunc_key_press_event(event) {
        if (event.get_key_symbol() === Clutter.KEY_Escape) {
            if (this._completions.visible)
                this._hideCompletions();
            else
                this.close();
            return Clutter.EVENT_STOP;
        }
        return super.vfunc_key_press_event(event);
    }

    _inputKey(event) {
        const key = event.get_key_symbol();
        const state = event.get_state();
        if ((key === Clutter.KEY_Return || key === Clutter.KEY_KP_Enter) && !(state & Clutter.ModifierType.SHIFT_MASK)) {
            this._submit();
            return Clutter.EVENT_STOP;
        }
        if ((key === Clutter.KEY_l || key === Clutter.KEY_L) && state & Clutter.ModifierType.CONTROL_MASK) {
            this.clear();
            return Clutter.EVENT_STOP;
        }
        if (key === Clutter.KEY_Tab && !(state & Clutter.ModifierType.SHIFT_MASK)) {
            this._complete();
            return Clutter.EVENT_STOP;
        }
        if ([Clutter.KEY_Up, Clutter.KEY_Down].includes(key) &&
            (!this._entry.get_text().includes('\n') || state & Clutter.ModifierType.MOD1_MASK)) {
            if (this._historyIndex === this._history.length)
                this._historyDraft = this._entry.get_text();
            this._historyIndex = Math.max(0, Math.min(this._history.length,
                this._historyIndex + (key === Clutter.KEY_Up ? -1 : 1)));
            this._entry.set_text(this._history[this._historyIndex] ?? this._historyDraft);
            this._entry.clutter_text.set_cursor_position(-1);
            return Clutter.EVENT_STOP;
        }
        return Clutter.EVENT_PROPAGATE;
    }

    _submit() {
        const source = this._entry.get_text().trim();
        if (!source)
            return;
        this._entry.set_text('');
        this._history = [...this._history.filter(item => item !== source), source].slice(-MAX_ROWS);
        this._historyIndex = this._history.length;
        this._historyDraft = '';
        global.settings.set_strv(HISTORY_KEY, this._history);
        this.evaluate(source).catch(error => this._appendValues('error', [error]));
        this._entry.grab_key_focus();
    }

    async evaluate(source) {
        const model = this._evaluator;
        const row = new St.BoxLayout({orientation: VERTICAL, style_class: 'gnoblin-console-result'});
        row.add_child(textLabel(`> ${source}`, 'gnoblin-console-source', true));
        const pending = textLabel('Pending...', 'gnoblin-console-hint');
        row.add_child(pending);
        this._appendRow(row);
        this._pending++;
        this._updateStatus();
        const result = await model.evaluate(source);
        this._pending--;
        if (this._destroyed || model !== this._evaluator)
            return result;
        this._updateStatus();
        if (!this._rows.has(row))
            return result;
        pending.destroy();
        const output = new St.BoxLayout({style_class: 'gnoblin-console-output'});
        output.add_child(new St.Label({text: `r(${result.id})`, style_class: 'gnoblin-console-index'}));
        output.add_child(this._valueActor(result.error ?? result.value, Boolean(result.error)));
        output.add_child(button('Copy', () => St.Clipboard.get_default().set_text(
            St.ClipboardType.CLIPBOARD, result.error?.stack ?? preview(result.value))));
        row.add_child(output);
        if (result.error?.stack) {
            const stack = textLabel(result.error.stack.slice(0, 6000), 'gnoblin-console-stack', true);
            stack.hide();
            row.add_child(button('Stack trace', () => stack.visible = !stack.visible));
            row.add_child(stack);
        }
        this._scrollBottom();
        return result;
    }

    _valueActor(value, error = false) {
        const label = textLabel(preview(value), error ? 'gnoblin-console-error' : 'gnoblin-console-value', !isInspectable(value));
        if (!isInspectable(value))
            return label;
        const link = new St.Button({child: label, x_expand: true, can_focus: true, style_class: 'gnoblin-console-object'});
        link.connect('clicked', () => this.inspectObject(value));
        return link;
    }

    _appendValues(level, values) {
        const row = new St.BoxLayout({orientation: VERTICAL, style_class: `gnoblin-console-log ${level === 'error' ? 'gnoblin-console-error' : ''}`});
        row.add_child(textLabel(level, 'gnoblin-console-index'));
        for (const value of values)
            row.add_child(this._valueActor(value, level === 'error'));
        this._appendRow(row);
    }

    _appendRow(row) {
        this._rows.add(row);
        this._transcript.add_child(row);
        while (this._transcript.get_n_children() > MAX_ROWS) {
            const first = this._transcript.get_first_child();
            this._rows.delete(first);
            first.destroy();
        }
        this._scrollBottom();
    }

    _scrollBottom() {
        if (this._scrollIdle || this._destroyed)
            return;
        this._scrollIdle = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._scrollIdle = 0;
            const adjustment = this._scroll.vadjustment;
            adjustment.value = Math.max(adjustment.lower, adjustment.upper - adjustment.page_size);
            return GLib.SOURCE_REMOVE;
        });
    }

    _updateStatus() {
        this._status.text = this._pending ? `${this._pending} pending` : 'Compositor context';
    }

    _clearTranscript() {
        this._rows.clear();
        this._transcript.destroy_all_children();
        this._properties.destroy_all_children();
        this._inspector.hide();
        this._inspectStack = [];
        this._updateStatus();
    }

    clear() {
        this._evaluator.clear();
    }

    reset() {
        this.clear();
        this._newEvaluator();
        this._welcome();
        this._entry.grab_key_focus();
    }

    inspectObject(value) {
        this._inspectStack.push(value);
        if (this._inspectStack.length > 30)
            this._inspectStack.shift();
        this._showProperties(value);
        return value;
    }

    _showProperties(value) {
        this._properties.destroy_all_children();
        this._properties.add_child(textLabel(preview(value), 'gnoblin-console-source', true));
        for (const property of this._evaluator.properties(value)) {
            const row = new St.BoxLayout({orientation: VERTICAL, style_class: 'gnoblin-console-property'});
            row.add_child(textLabel(property.name, 'gnoblin-console-property-name'));
            row.add_child(property.accessor
                ? textLabel(property.preview, 'gnoblin-console-hint')
                : this._valueActor(property.value));
            this._properties.add_child(row);
        }
        this._inspector.show();
    }

    _complete() {
        const text = this._entry.get_text();
        const position = this._entry.clutter_text.get_cursor_position();
        const cursor = position < 0 ? text.length : [...text].slice(0, position).join('').length;
        const completion = this._evaluator.complete(text, cursor);
        if (!completion.items.length) {
            this._status.text = 'No completions';
            return;
        }
        const accept = candidate => {
            const item = typeof candidate === 'string' ? candidate : candidate.value;
            this._entry.set_text(text.slice(0, completion.start) + item + text.slice(completion.end));
            this._entry.clutter_text.set_cursor_position([...text.slice(0, completion.start) + item].length);
            this._hideCompletions();
            this._entry.grab_key_focus();
        };
        if (completion.items.length === 1) {
            accept(completion.items[0]);
            return;
        }
        this._hideCompletions();
        for (const item of completion.items.slice(0, 6))
            this._completions.add_child(button(item.label, () => accept(item)));
        if (completion.items.length > 6)
            this._completions.add_child(textLabel(`+${completion.items.length - 6} matches; type to narrow`, 'gnoblin-console-hint'));
        this._completions.show();
    }

    _hideCompletions() {
        this._completions.hide();
        this._completions.destroy_all_children();
    }
});
