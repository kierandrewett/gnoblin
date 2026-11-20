/** What the compositor answered to a verb: the operation's value (parsed from its JSON, `undefined` for a bare done) or its error code. */
export interface VerbAnswer {
    readonly value: unknown;
    readonly error: string | null;
}
/** A signal registration. A reload removes what a generation registered; `unsubscribe` removes one sooner. */
export declare class VerbSubscription {
    #private;
    constructor(registration: number);
    unsubscribe(): void;
}
/** The `config` object of the `gnoblin` namespace. */
export declare class ConfigVerbs {
    static readonly CLASS = "gnoblin.config";
    /** Calls `callback` each time the `reload_failed` signal is raised, with its payload in the schema's field order. */
    onReloadFailed(callback: (path: string, error: string, sequence: number, time: number) => void): VerbSubscription;
    /** Calls `callback` each time the `reloaded` signal is raised, with its payload in the schema's field order. */
    onReloaded(callback: (path: string, revision: number, warnings: string[], notApplied: string[], sequence: number, time: number) => void): VerbSubscription;
}
/** The `eis` object of the `gnoblin` namespace. */
export declare class EisVerbs {
    static readonly CLASS = "gnoblin.eis";
    /** Calls `open` on the compositor's `eis` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    open(session: number, owner: string, capabilities: number, path: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `close` on the compositor's `eis` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    close(session: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
}
/** The `idle` object of the `gnoblin` namespace. */
export declare class IdleVerbs {
    static readonly CLASS = "gnoblin.idle";
    /** Calls `inhibit` on the compositor's `idle` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    inhibit(holder: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `uninhibit` on the compositor's `idle` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    uninhibit(holder: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
}
/** The `operation` object of the `gnoblin` namespace. */
export declare class OperationVerbs {
    static readonly CLASS = "gnoblin.operation";
    /** Calls `callback` each time the `completed` signal is raised, with its payload in the schema's field order. */
    onCompleted(callback: (operationId: number, method: string, ok: boolean, error: string, sequence: number, time: number) => void): VerbSubscription;
}
/** The `output` object of the `gnoblin` namespace. */
export declare class OutputVerbs {
    static readonly CLASS = "gnoblin.output";
    /** Calls `apply` on the compositor's `output` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    apply(x?: number, y?: number, scale?: number, transform?: number, enabled?: boolean, mode?: string, supersample?: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `callback` each time the `closed` signal is raised, with its payload in the schema's field order. */
    onClosed(callback: () => void): VerbSubscription;
    /** Calls `callback` each time the `added` signal is raised, with its payload in the schema's field order. */
    onAdded(callback: (outputId: string, output: Record<string, unknown>) => void): VerbSubscription;
    /** Calls `callback` each time the `changed` signal is raised, with its payload in the schema's field order. */
    onChanged(callback: (outputId: string, output: Record<string, unknown>, changed: string[]) => void): VerbSubscription;
    /** Calls `callback` each time the `removed` signal is raised, with its payload in the schema's field order. */
    onRemoved(callback: (outputId: string, last: Record<string, unknown>) => void): VerbSubscription;
}
/** The `portal_ui` object of the `gnoblin` namespace. */
export declare class PortalUiVerbs {
    static readonly CLASS = "gnoblin.portal_ui";
    /** Calls `file_chooser` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    fileChooser(requestId: number, appId: string, deadlineMs: number, mode: string, title: string, multiple: boolean, filterNames: string[], filterPatterns: string[], currentFilter: number, currentFolder: string, currentName: string, choiceIds: string[], choiceLabels: string[], onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `app_chooser` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    appChooser(requestId: number, appId: string, deadlineMs: number, contentType: string, lastChoice: string, filterText: string, choiceIds: string[], choiceLabels: string[], onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `print` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    print(requestId: number, appId: string, deadlineMs: number, title: string, choiceIds: string[], choiceLabels: string[], copies: number, maxCopies: number, pages: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `account` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    account(requestId: number, appId: string, deadlineMs: number, reason: string, name: string, avatarUri: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `wallpaper` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    wallpaper(requestId: number, appId: string, deadlineMs: number, uri: string, options: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `background` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    background(requestId: number, appId: string, deadlineMs: number, reason: string, autostartRequested: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `dynamic_launcher` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    dynamicLauncher(requestId: number, appId: string, deadlineMs: number, name: string, icon: string, desktopFileId: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `screen_cast` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    screenCast(requestId: number, appId: string, deadlineMs: number, sourceTypes: number, cursorMode: number, multiple: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `remote_desktop` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    remoteDesktop(requestId: number, appId: string, deadlineMs: number, deviceTypes: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `input_capture` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    inputCapture(requestId: number, appId: string, deadlineMs: number, capabilities: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `screenshot` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    screenshot(requestId: number, appId: string, deadlineMs: number, interactive: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `pick_color` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    pickColor(requestId: number, appId: string, deadlineMs: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `secret` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    secret(requestId: number, appId: string, deadlineMs: number, label: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `cancel` on the compositor's `portal_ui` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    cancel(requestId: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
}
/** The `portal_outcome` object of the `gnoblin` namespace. */
export declare class PortalOutcomeVerbs {
    static readonly CLASS = "gnoblin.portal_outcome";
    /** Calls `handled` on the compositor's `portal_outcome` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    handled(requestId: number, uris: string[], choice: string, remember: boolean, copies: number, pages: string, grantBits: number, cursorMode: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `cancelled` on the compositor's `portal_outcome` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    cancelled(requestId: number, reason: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `declined` on the compositor's `portal_outcome` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    declined(requestId: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `error` on the compositor's `portal_outcome` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    error(requestId: number, code: string, message: string, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
}
/** The `request` object of the `gnoblin` namespace. */
export declare class RequestVerbs {
    static readonly CLASS = "gnoblin.request";
    /** Calls `claim` on the compositor's `request` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    claim(request: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `decline` on the compositor's `request` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    decline(request: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `callback` each time the `offered` signal is raised, with its payload in the schema's field order. */
    onOffered(callback: (request: number, deadline: number, slot: string, title: string, message: string, subject: Uint8Array) => void): VerbSubscription;
    /** Calls `callback` each time the `finished` signal is raised, with its payload in the schema's field order. */
    onFinished(callback: (request: number, completion: string, reason: string) => void): VerbSubscription;
}
/** The `session` object of the `gnoblin` namespace. */
export declare class SessionVerbs {
    static readonly CLASS = "gnoblin.session";
    /** Calls `logout` on the compositor's `session` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    logout(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `poweroff` on the compositor's `session` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    poweroff(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `reboot` on the compositor's `session` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    reboot(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `suspend` on the compositor's `session` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    suspend(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `hibernate` on the compositor's `session` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    hibernate(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `callback` each time the `ready` signal is raised, with its payload in the schema's field order. */
    onReady(callback: () => void): VerbSubscription;
    /** Calls `callback` each time the `bell` signal is raised, with its payload in the schema's field order. */
    onBell(callback: (count: number) => void): VerbSubscription;
    /** Calls `callback` each time the `reload_requested` signal is raised, with its payload in the schema's field order. */
    onReloadRequested(callback: (generation: number) => void): VerbSubscription;
    /** Calls `callback` each time the `lock_state_changed` signal is raised, with its payload in the schema's field order. */
    onLockStateChanged(callback: (state: string, revision: number) => void): VerbSubscription;
}
/** The `window` object of the `gnoblin` namespace. */
export declare class WindowVerbs {
    static readonly CLASS = "gnoblin.window";
    /** Calls `focus` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    focus(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `present` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    present(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `kill` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    kill(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `close` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    close(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_minimized` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setMinimized(value: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_maximized` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setMaximized(value: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_fullscreen` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setFullscreen(value: boolean, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `move` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    move(x: number, y: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `resize` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    resize(width: number, height: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_opacity` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setOpacity(opacity: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_scale` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setScale(scale: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_offset` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setOffset(x: number, y: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `reset_visuals` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    resetVisuals(onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_workspace` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setWorkspace(workspace: Record<string, unknown>, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `set_output` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    setOutput(output: Record<string, unknown>, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `request_menu` on the compositor's `window` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */
    requestMenu(x: number, y: number, width: number, height: number, onAnswer?: (answer: VerbAnswer) => void, object?: number): void;
    /** Calls `callback` each time the `closed` signal is raised, with its payload in the schema's field order. */
    onClosed(callback: () => void): VerbSubscription;
    /** Calls `callback` each time the `created` signal is raised, with its payload in the schema's field order. */
    onCreated(callback: (window: Record<string, unknown>) => void): VerbSubscription;
    /** Calls `callback` each time the `changed` signal is raised, with its payload in the schema's field order. */
    onChanged(callback: (windowId: number, changed: string[], window: Record<string, unknown>) => void): VerbSubscription;
    /** Calls `callback` each time the `menu_requested` signal is raised, with its payload in the schema's field order. */
    onMenuRequested(callback: (x: number, y: number, width: number, height: number, source: string, seat: string) => void): VerbSubscription;
}
/** The `workspace` object of the `gnoblin` namespace. */
export declare class WorkspaceVerbs {
    static readonly CLASS = "gnoblin.workspace";
    /** Calls `callback` each time the `created` signal is raised, with its payload in the schema's field order. */
    onCreated(callback: (workspace: Record<string, unknown>) => void): VerbSubscription;
    /** Calls `callback` each time the `removed` signal is raised, with its payload in the schema's field order. */
    onRemoved(callback: (workspaceId: number, last: Record<string, unknown>) => void): VerbSubscription;
}
/** The compositor's `gnoblin` namespace. */
export declare const verbs: {
    config: ConfigVerbs;
    eis: EisVerbs;
    idle: IdleVerbs;
    operation: OperationVerbs;
    output: OutputVerbs;
    portalUi: PortalUiVerbs;
    portalOutcome: PortalOutcomeVerbs;
    request: RequestVerbs;
    session: SessionVerbs;
    window: WindowVerbs;
    workspace: WorkspaceVerbs;
};
