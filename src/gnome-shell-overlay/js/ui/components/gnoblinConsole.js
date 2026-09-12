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
import {ConsoleEvaluator, LuaConsoleEvaluator, LuaValue, isInspectable, preview} from './gnoblinConsoleEvaluator.js';

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
    const actor = new St.Button({label, style_class: styleClass, can_focus: true, x_align: Clutter.ActorAlign.FILL});
    actor.get_child().x_align = Clutter.ActorAlign.START;
    actor.get_child().x_expand = true;
    actor.connect_after('style-changed', () => actor.get_child().set_line_alignment(Pango.Alignment.LEFT));
    actor.get_child().set_line_alignment(Pango.Alignment.LEFT);
    actor.connect('clicked', action);
    return actor;
}

function highlight(actor, source, language = 'js') {
    const attributes = new Pango.AttrList();
    const base = Pango.attr_foreground_new(0xdddd, 0xdddd, 0xdddd);
    base.start_index = 0;
    base.end_index = 0xffffffff;
    attributes.insert(base);
    const tokens = /(?:\/\/[^\n]*|\/\*[\s\S]*?(?:\*\/|$))|(?:"(?:\\.|[^"\\])*"?|'(?:\\.|[^'\\])*'?|`(?:\\.|[^`\\])*`?)|\b(?:const|let|var|function|class|return|throw|new|await|async|if|else|for|while|try|catch|typeof|instanceof|true|false|null|undefined|local|end|then|do|elseif|repeat|until|and|or|not|nil|in)\b|\b(?:0x[\da-f]+|\d+(?:\.\d+)?)\b/gi;
    const pattern = language === 'lua' ? new RegExp('--[^\\n]*|' + tokens.source, 'g') : tokens;
    for (const match of source.matchAll(pattern)) {
        const token = match[0];
        const color = (token.startsWith('/') || token.startsWith('--')) ? [138, 161, 126]
            : /^["'`]/.test(token) ? [233, 166, 145]
                : /^\d/.test(token) ? [153, 201, 255] : [197, 165, 232];
        const attribute = Pango.attr_foreground_new(...color.map(channel => channel * 257));
        attribute.start_index = new TextEncoder().encode(source.slice(0, match.index)).length;
        attribute.end_index = attribute.start_index + new TextEncoder().encode(token).length;
        attributes.change(attribute);
    }
    actor.set_attributes(attributes);
}

export const DeveloperConsole = GObject.registerClass(
class DeveloperConsole extends St.Widget {
    _init() {
        super._init({name: 'gnoblin-developer-console', visible: false, reactive: true});
        this._open = false;
        this.connect('captured-event', (_actor, event) => this._capturedEvent(event));
        this._destroyed = false;
        this._expanded = false;
        this._pending = 0;
        this._rows = new Set();
        this._history = global.settings.get_strv(HISTORY_KEY).slice(-MAX_ROWS);
        this._historyIndex = this._history.length;
        this._historyDraft = '';

        this._panel = new St.BoxLayout({
            style_class: 'gnoblin-console', orientation: VERTICAL, reactive: true,
        });
        this.add_child(this._panel);
        this._tabs = new St.BoxLayout({style_class: 'gnoblin-console-tabs'});
        this._languageTabs = new Map();
        this._drafts = {js: '', lua: ''};
        for (const [language, label] of [['js', 'JavaScript'], ['lua', 'Lua']]) {
            const tab = button(label, () => this._selectLanguage(language), 'gnoblin-console-tab');
            tab.x_expand = false;
            tab.get_child().x_expand = false;
            this._languageTabs.set(language, tab);
            this._tabs.add_child(tab);
        }
        this._panel.add_child(this._tabs);
        this._body = new St.BoxLayout({y_expand: true, style_class: 'gnoblin-console-body'});
        this._transcript = new St.BoxLayout({orientation: VERTICAL, x_expand: true});
        this._scroll = new St.ScrollView({
            x_expand: true, y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
        });
        this._body.add_child(this._scroll);
        this._panel.add_child(this._body);
        this._flow = new St.BoxLayout({orientation: VERTICAL, x_expand: true});
        this._flow.add_child(this._transcript);
        this._input = new St.BoxLayout({style_class: 'gnoblin-console-input'});
        this._prompt = new St.Label({text: '›', style_class: 'gnoblin-console-prompt'});
        this._input.add_child(this._prompt);
        this._entry = new St.Entry({
            style_class: 'gnoblin-console-entry', x_expand: true, can_focus: true,
            accessible_name: 'JavaScript',
        });
        this._entry.clutter_text.set_single_line_mode(false);
        this._entry.clutter_text.set_activatable(false);
        this._entry.clutter_text.set_line_wrap(true);
        this._entry.clutter_text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR);
        this._entry.clutter_text.connect('key-press-event', (_text, event) => this._inputKey(event));
        this._entry.connect_after('style-changed', () => highlight(this._entry.clutter_text, this._entry.get_text(), this._language));
        this._entry.clutter_text.connect('text-changed', () => {
            highlight(this._entry.clutter_text, this._entry.get_text(), this._language);
            this._queueCompletion();
        });
        this._entry.clutter_text.connect('notify::cursor-position', () => this._queueCompletion());
        this._input.add_child(this._entry);
        this._flow.add_child(this._input);
        this._completions = new St.BoxLayout({
            orientation: VERTICAL, style_class: 'gnoblin-console-completions',
            visible: false, x_align: Clutter.ActorAlign.START,
        });
        this._flow.add_child(this._completions);
        this._scroll.set_child(this._flow);

        this._newEvaluator();
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
            if (this._completionIdle)
                GLib.source_remove(this._completionIdle);
            this._rows.clear();
            this._evaluator.clear();
            this._luaEvaluator?.reset();
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
        this._jsEvaluator = evaluator;
        this._luaEvaluator ??= new LuaConsoleEvaluator(
            (operation, source) => Meta.gnoblin_console_lua(operation, source).recursiveUnpack(),
            line => this._appendRow(textLabel(line, 'gnoblin-console-log', true)));
        this._language = 'js';
        this._evaluator = evaluator;
        this._languageTabs.get('js').add_style_pseudo_class('selected');
        this._languageTabs.get('lua').remove_style_pseudo_class('selected');
    }

    _selectLanguage(language) {
        if (!this._languageTabs.has(language))
            return;
        this._drafts[this._language] = this._entry.get_text();
        this._language = language;
        this._evaluator = language === 'lua' ? this._luaEvaluator : this._jsEvaluator;
        for (const [name, tab] of this._languageTabs) {
            if (name === language)
                tab.add_style_pseudo_class('selected');
            else
                tab.remove_style_pseudo_class('selected');
        }
        this._entry.accessible_name = language === 'lua' ? 'Lua' : 'JavaScript';
        this._entry.set_text(this._drafts[language]);
        highlight(this._entry.clutter_text, this._entry.get_text(), language);
        this._hideCompletions();
        this._entry.grab_key_focus();
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
    }

    _capturedEvent(event) {
        if (event.type() !== Clutter.EventType.KEY_PRESS)
            return Clutter.EVENT_PROPAGATE;
        const key = event.get_key_symbol();
        const altF2 = key === Clutter.KEY_F2 &&
            (event.get_state() & Clutter.ModifierType.MOD1_MASK);
        // Dismiss before the entry, completion buttons or inspector consume it.
        if (key === Clutter.KEY_Escape || altF2) {
            this.close();
            return Clutter.EVENT_STOP;
        }
        return Clutter.EVENT_PROPAGATE;
    }

    _inputKey(event) {
        const key = event.get_key_symbol();
        const state = event.get_state();
        if (this._completions.visible && [Clutter.KEY_Up, Clutter.KEY_Down].includes(key)) {
            this._selectCompletion(this._completionIndex + (key === Clutter.KEY_Up ? -1 : 1));
            return Clutter.EVENT_STOP;
        }
        if (key === Clutter.KEY_Tab && this._completions.visible) {
            this._acceptCompletion();
            return Clutter.EVENT_STOP;
        }
        if ((key === Clutter.KEY_Return || key === Clutter.KEY_KP_Enter) &&
            this._completions.visible && !(state & Clutter.ModifierType.SHIFT_MASK)) {
            this._acceptCompletion();
            return Clutter.EVENT_STOP;
        }
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
        if (source === ':reset') {
            this._luaEvaluator.reset();
            this._newEvaluator();
            this._selectLanguage('js');
            this.clear();
            return {source, value: undefined, error: null, id: 0};
        }
        const language = this._language;
        const model = this._evaluator;
        const row = new St.BoxLayout({orientation: VERTICAL, style_class: 'gnoblin-console-result'});
        const command = textLabel(`› ${source}`, 'gnoblin-console-source', true);
        command.connect_after('style-changed', () => highlight(command.clutter_text, `› ${source}`, language));
        highlight(command.clutter_text, `› ${source}`, language);
        row.add_child(command);
        const pending = textLabel('Pending...', 'gnoblin-console-hint');
        row.add_child(pending);
        this._appendRow(row);
        this._pending++;
        const result = await model.evaluate(source);
        this._pending--;
        if (this._destroyed || model !== this._evaluator)
            return result;
        if (!this._rows.has(row))
            return result;
        pending.destroy();
        const output = new St.BoxLayout({style_class: 'gnoblin-console-output'});
        if (result.error) {
            output.add_style_class_name('gnoblin-console-error');
            const detail = textLabel(result.error.stack?.slice(0, 6000) ?? '', 'gnoblin-console-stack', true);
            detail.hide();
            output.add_child(button(`▸ Uncaught ${preview(result.error)}`, () => {
                detail.visible = !detail.visible;
                this._scrollBottom();
            }, 'gnoblin-console-error-text'));
            row.add_child(output);
            row.add_child(detail);
        } else {
            if (result.lua) {
                for (const value of result.items ?? [])
                    output.add_child(this._valueActor(value, false, new Set(), model));
            } else {
                output.add_child(this._valueActor(result.value, false, new Set(), model));
            }
            row.add_child(output);
        }
        this._scrollBottom();
        return result;
    }

    _valueActor(value, error = false, ancestors = new Set(), model = this._evaluator, receiver = value) {
        const summary = preview(value);
        const kind = value instanceof LuaValue ? value.kind : typeof value;
        const label = textLabel(summary, error ? 'gnoblin-console-error-text' : `gnoblin-console-value ${kind}`, true);
        if (!isInspectable(value))
            return label;
        if (ancestors.has(value)) {
            label.text = `↩ ${summary}`;
            return label;
        }
        const path = new Set(ancestors).add(value);
        const group = new St.BoxLayout({orientation: VERTICAL, x_expand: true});
        let properties = null;
        const appendPage = offset => {
            for (const property of model.properties(value, offset, 100, receiver)) {
                if (property.more !== undefined) {
                    const more = button(property.name, () => {
                        more.destroy();
                        appendPage(property.more);
                    }, 'gnoblin-console-more');
                    properties.add_child(more);
                    continue;
                }
                const row = new St.BoxLayout({style_class: 'gnoblin-console-property'});
                const name = new St.Label({text: `${property.name}: `,
                    style_class: property.enumerable === false ? 'gnoblin-console-property-name non-enumerable' : 'gnoblin-console-property-name',
                    accessible_name: `${property.name}${property.flags ? ` (${property.flags})` : ''}`});
                if (property.key)
                    row.add_child(this._valueActor(property.key, false, path, model));
                else
                    row.add_child(name);
                if (property.accessor) {
                    if (property.read) {
                        const getter = button(property.preview, () => {
                            getter.destroy();
                            try {
                                row.add_child(this._valueActor(property.read(), false, path, model));
                            } catch (failure) {
                                row.add_child(textLabel(`${failure.name}: ${failure.message}`, 'gnoblin-console-error-text', true));
                            }
                        }, 'gnoblin-console-getter');
                        getter.accessible_name = `Evaluate getter ${property.name}`;
                        row.add_child(getter);
                    } else {
                        row.add_child(textLabel(property.preview, 'gnoblin-console-hint'));
                    }
                } else {
                    row.add_child(this._valueActor(property.value, false, path, model, property.receiver ?? property.value));
                }
                properties.add_child(row);
            }
            if (!properties.get_n_children())
                properties.add_child(textLabel('(no own properties)', 'gnoblin-console-hint'));
        };
        const toggle = button(`▸ ${summary}`, () => {
            if (!properties) {
                properties = new St.BoxLayout({orientation: VERTICAL, style_class: 'gnoblin-console-properties'});
                group.add_child(properties);
                appendPage(0);
            } else {
                properties.visible = !properties.visible;
            }
            toggle.label = `${properties.visible ? '▾' : '▸'} ${summary}`;
        }, 'gnoblin-console-object');
        toggle.accessible_name = `Expand ${summary}`;
        group.add_child(toggle);
        return group;
    }

    _appendValues(level, values) {
        const row = new St.BoxLayout({orientation: VERTICAL, style_class: `gnoblin-console-log ${level === 'error' ? 'gnoblin-console-error' : ''}`});
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

    _clearTranscript() {
        this._rows.clear();
        this._transcript.destroy_all_children();
    }

    clear() {
        this._evaluator.clear();
        this._clearTranscript();
    }

    reset() {
        this.clear();
        this._newEvaluator();
        this._entry.grab_key_focus();
    }

    inspectObject(value) {
        const row = new St.BoxLayout({orientation: VERTICAL});
        const actor = this._valueActor(value);
        row.add_child(actor);
        this._appendRow(row);
        if (isInspectable(value))
            actor.get_first_child().emit('clicked', 1);
        return value;
    }

    _queueCompletion() {
        if (this._completionIdle || this._destroyed)
            return;
        this._completionIdle = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._completionIdle = 0;
            if (!this._suppressCompletion)
                this._complete();
            this._suppressCompletion = false;
            return GLib.SOURCE_REMOVE;
        });
    }

    _complete() {
        this._hideCompletions();
        const text = this._entry.get_text();
        const position = this._entry.clutter_text.get_cursor_position();
        const cursor = position < 0 ? text.length : [...text].slice(0, position).join('').length;
        const completion = this._evaluator.complete(text, cursor);
        if (!completion.items.length || !this._open)
            return;
        this._completion = {...completion, text};
        for (const [index, item] of completion.items.slice(0, 12).entries()) {
            const choice = button(item.label, () => {
                this._completionIndex = index;
                this._acceptCompletion();
            }, 'gnoblin-console-completion');
            this._completions.add_child(choice);
        }
        this._completions.show();
        this._selectCompletion(0);
        this._scrollBottom();
    }

    _selectCompletion(index) {
        const children = this._completions.get_children();
        this._completionIndex = (index + children.length) % children.length;
        children.forEach((child, i) => {
            if (i === this._completionIndex)
                child.add_style_pseudo_class('selected');
            else
                child.remove_style_pseudo_class('selected');
        });
    }

    _acceptCompletion() {
        const {text, start, end, items} = this._completion;
        const value = items[this._completionIndex].value;
        this._suppressCompletion = true;
        this._entry.set_text(text.slice(0, start) + value + text.slice(end));
        this._entry.clutter_text.set_cursor_position([...text.slice(0, start) + value].length);
        this._hideCompletions();
        this._entry.grab_key_focus();
    }

    _hideCompletions() {
        this._completions.hide();
        this._completions.destroy_all_children();
    }
});
