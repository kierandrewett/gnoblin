# Gnoblin JavaScript Environment — Complete API & Runtime Specification

**Status:** Proposed design, not current implementation  
**Version:** 0.1-draft · 13 November 2026  
**Target:** `kierandrewett/gnoblin`, branch `rust-compositor`  
**Audience:** Gnoblin maintainers, implementers, configuration authors and extension developers

> This document consolidates the API decisions and examples developed in the design conversation. All JavaScript interfaces are **proposed** unless explicitly marked as existing Rust/Lua infrastructure. Examples are normative for **intended API behaviour**, not claims that the current Gnoblin build executes them.

## 1. Design goals and firm decisions

Gnoblin is a **3D-first, fully programmable Wayland compositor**. A flat desktop is an orthographic view of the same native scene system used for perspective window layouts and XR. Users configure and extend it using **one modern JavaScript language**, with ordinary classes, ES modules, live native objects, typed listeners and an optional native `.so` extension mechanism.

1. `gnoblin` is a built-in global; `import { Client, Workspace, ... } from "gnoblin"` loads built-in classes. No Node.js, npm, package manager, TypeScript build step, HTML or DOM is required for a config.
2. A **`Client`** is a managed toplevel window. It is **not** a Wayland `wl_client` connection. One Wayland application process/connection may own multiple `Client` objects.
3. `gnoblin.clients`, `gnoblin.workspaces`, `gnoblin.outputs`, `gnoblin.displays` and `gnoblin.input.contexts` are real, live `Collection<T>` objects. `workspace.clients` is the workspace's own live collection of the _same_ `Client` instances.
4. `client.style` means **one client's style**. `gnoblin.clients.style` means inherited global defaults; `workspace.clients.style` means workspace defaults. Styles, borders, shadows and filters are typed native-backed **objects**, with CSS-like names and ordered effect stacks but **no required CSS stylesheet language**.
5. `Workspace` owns managed clients and a layout; `Scene` owns the visual graph; `View` is a scene node referencing other content; `Viewport` looks at content with `Camera2D` or `Camera3D`; `Output` is hardware; `Display` arranges one or more outputs for presentation.
6. `gnoblin.requestTransition(() => { ... }, options)` is the universal transition mechanism. The callback runs **once** to stage state changes; the native engine interpolates their visual effects, including geometry, styles, layouts and 2D↔3D camera changes. Ordinary property assignment is immediate. The optional `object.set({...}, options)` is shorthand for an atomic change to one object.
7. Every object emits typed domain events through listener **classes** (`ClientListener`, `OutputListener`, `CollectionListener`, etc.), rather than dotted event-name strings. A collection can forward member listeners for all existing _and future_ members.
8. `gnoblin.input.primary` is an `InputContext` (public replacement for Wayland's _seat_). It groups physical input devices, keyboard/pointer state and focus. Input bindings are OOP classes; `.bind()` is also available for simple callbacks.
9. Every built-in `Layout` **registers its own default `Binding` instances**. Constructor `triggers` override those defaults; omitting an option preserves a default; `null` disables it. `layout.bindings` can be inspected, changed, removed or replaced.
10. Built-in and user-created input bindings **can be implemented entirely in JavaScript** using `onStart/onUpdate/onEnd/onCancel`. Rust provides input delivery, capture, scene maths, safe transactions, rendering and Wayland protocol handling; **no `beginNativeGrab()` shortcut is required**.
11. XR lives under **`gnoblin.extensions.xr`** and in optional module `gnoblin/extensions/xr`; an optional native `.so` supplies `XRCamera3D`, `XRViewport`, XR sessions, devices and XR binding classes. XR-specific features are not added to core `Camera3D` or `Viewport`.
12. XR panel manipulation changes the **`View`** in real time, not the source `Client`. **On release**, resizing may request a new `Client` logical buffer size if the panel becomes too large or its aspect ratio changes. The request affects the _single underlying toplevel_, even when shown in several views.
13. The compositor stays in Rust/Smithay; JavaScript and optional native extensions execute outside its critical thread by default. The scripting experience must feel integrated without making the renderer depend on a JS handler arriving every frame.
14. User-written classes have **ordinary JavaScript fields** (`this.radius = 900`), not `static properties` schemas or user-written `Proxy`/`Reflect` plumbing. The host must explicitly define how observable fields are intercepted, and cannot assume Rust alone can see arbitrary JS writes.

### Source architecture already present

The present repository already has much of the foundation: `gnoblin-render` stores 3D transforms, cameras, effects, native tween tracks and transform-consistent hit-testing; `gnoblin-output` performs transactional display configuration; `gnoblin-render::transaction` validates atomic tree changes; the current stock shell implements floating/tiling/XR policy in Lua; and `gnoblin-xr` currently runs as an optional external process. These are **starting points**, not an assertion that this JavaScript API exists today.

Relevant source: [`gnoblin-render`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/crates/gnoblin-render/src/lib.rs), [`render transaction`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/crates/gnoblin-render/src/transaction.rs), [`output engine`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/crates/gnoblin-output/src/lib.rs), [`floating.lua`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/data/lua/gnoblin/shell/floating.lua), [`tiling.lua`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/data/lua/gnoblin/shell/tiling.lua), [`XR docs`](https://github.com/kierandrewett/gnoblin/blob/rust-compositor/docs/xr.md).

---

## 2. Architecture and naming

```text
User JS ES modules (one privileged, restartable runtime)
  ├─ gnoblin global: clients, workspaces, displays, outputs, input, scene ...
  ├─ stock JS library: layouts, bindings, style classes, listeners
  ├─ user classes, event handlers, configuration and hot reload
  └─ optional extension bindings (gnoblin/extensions/xr, etc.)
                      │
                      │ typed object API / revisions / transactions / input updates
                      ▼
Rust compositor: object store + immutable snapshots + validation
  ├─ scene and transform graph; GPU rendering and damage
  ├─ Wayland protocol, client configure and focus safety
  ├─ output/modeset transaction engine
  ├─ transition/tween player and native property adapters
  └─ input/gesture capture and ray hit-testing
                      ▲
                      │ versioned extension IPC
Optional extension host ─ dlopen(ext.so) ─ OpenXR runtime
  ├─ session, tracking, controllers, swapchains
  └─ layer submission of Gnoblin-produced views
```

**Object relationship diagram:**

```text
gnoblin
├── clients: Collection<Client>                (all managed toplevels)
├── workspaces: Collection<Workspace>
│   └── Workspace : Node
│       ├── clients: Collection<Client>        (membership, not copies)
│       ├── layout: Layout
│       └── children: Collection<Node>
├── scene: Scene
│   └── root: Node → Workspace, Client, View, Camera, other Nodes
├── outputs: Collection<Output>                 (physical/virtual connectors)
│   └── profiles: Collection<OutputProfile>    (persistent hardware policy)
├── displays: Collection<Display>               (one/many outputs + one viewport)
│   └── viewport: Viewport
│       ├── content: Node                      (often Workspace)
│       └── camera: Camera2D | Camera3D
├── input: InputManager
│   ├── contexts: Collection<InputContext>    (wl_seat equivalents)
│   ├── devices: Collection<InputDevice>
│   └── primary: InputContext
├── extensions: ExtensionRegistry
│   └── xr?: XRExtension                    (registered only if available)
└── requestTransition(...): Transition
```

`Scene`, `Node` and `View` concern content and space. `Viewport` concerns observation and projection. `Display` concerns presenting one viewport across hardware. `Output` concerns a connector and its display mode. **An `Output` does not own a workspace or camera; `Display.viewport` does.** This supersedes earlier prototype examples using `output.viewport`.

### Scene graph vs view references

`Node` children form an ownership tree. A `View` is a _node in that tree_, but references content elsewhere without moving/reparenting it. Multiple views can reference the same client or workspace. The renderer rejects cyclic view references and refuses prohibited secure-content duplication. Layouts transform scene nodes; cameras transform their presentation, rather than placing clients.

### Coordinate systems

- Normal desktop scenes use logical **pixels** and a default `Camera2D`, orthographic projection.
- General `Camera3D` uses a scene's documented unit scale (by default logical units, with explicit `scene.metresPerUnit` when physical scale matters).
- XR spatial rooms conventionally use **metres**. `XRViewport` and `XRCamera3D` convert to/from native OpenXR reference spaces; XR panels specify physical `View.size` in metres within that room.
- Rotations use **radians** unless a type explicitly indicates otherwise; 3D rotations should expose quaternion math for reliable interpolation. APIs never silently mix local, parent and world space; the event tells the developer which space it uses.
- `bounds` denotes flat content bounds; `transform` denotes the native 3D transform. Layouts may return both.

---

## 3. Runtime, modules and global object

**Configuration file:** `~/.config/gnoblin/init.js`, automatically loaded by the privileged JS runtime. Modules under the same directory can be imported with relative ES module syntax. Shipped standard modules are imported by bare specifier `gnoblin` or `gnoblin/<module>`; extensions by `gnoblin/extensions/<id>`. No Node.js APIs, npm installation, HTML page, `document` or browser `window` global are required.

```javascript
// ~/.config/gnoblin/init.js
import { Client, Workspace, FloatingLayout, ClientListener } from "gnoblin";
import { DevelopmentWorkspace } from "./workspaces/DevelopmentWorkspace.js";

const development = new DevelopmentWorkspace();
gnoblin.workspaces.add(development);
gnoblin.clients.style.cornerRadius = 12;
gnoblin.input.primary.bind("Super+Return", () => gnoblin.spawn("ghostty"));
```

`gnoblin` is global to every loaded user ES module and references **one** session object. ES imports return types, helpers, built-in policies and extension APIs. The JS environment is sandboxed by default: filesystem access is limited to explicit module/config loading and granted services; arbitrarily loading `.so` from user scripts is prohibited. Native extension loading is done by the extension manager, not `dlopen()` from untrusted JS.

An embedded QuickJS/SpiderMonkey-class engine could implement this. **The public specification does not require one particular engine**; it does require ES classes, modern modules, stable host objects, type declarations, interruptible execution and input latency testing. Optional TypeScript tooling is _editor-side only_: the runtime executes JS, and Gnoblin ships generated `.d.ts` declarations.

User configuration is executable behaviour, **not** one language for config plus another language for scripting. Bingux is an independent Wayland layer-shell application and need not share Gnoblin's JS runtime, rendering API or components.

---

## 4. Core object identity and typed listener interfaces

All native objects have stable IDs, explicit lifetime and typed properties; wrappers for the same native ID compare as the same JS object within a runtime generation. Objects can be inserted into multiple **reference collections**, but there is only one owner for scene-node lifetime. A destroyed object becomes `alive === false`; writes and methods on it reject cleanly.

```typescript
abstract class NativeObject {
    readonly id: string;
    readonly alive: boolean;
    set(properties: object, options?: TransitionOptions): Transition | void;
    addListener(listener: object): void;
    removeListener(listener: object): void;
}

class Node extends NativeObject {
    readonly parent: Node | null;
    readonly children: Collection<Node>;
    transform: Transform3D;
    opacity: number;
    visible: boolean;
    style: Style;
    layout?: Layout | null; // for layout-bearing scene containers (Workspace, XR room)
    position: Vec3; // convenience accessor for transform.position
    rotation: Quaternion | Euler3; // convenience accessor for transform.rotation
    scale: number | Vec3; // convenience accessor for transform.scale
    add(node: Node): void;
    remove(node: Node): void;
    destroy(): void;
}

class Client extends Node {
    // Managed Wayland *toplevel*, not wl_client connection
    readonly appId: string;
    readonly title: string;
    readonly committedSize: Size2D;
    size: Size2D; // desired client logical size / configure request
    bounds: Rect; // layout geometry
    readonly workspace: Workspace | null;
    floatingBounds?: Rect; // user-managed floating placement when applicable
    readonly fullscreen: boolean;
    style: ClientStyle;
    readonly computedStyle: Readonly<ClientStyle>;
    close(): void;
}
```

Listener types are **classes with optional overridable methods**. The generated TypeScript declarations provide correct signatures. Every method's first argument is its originating native object, regardless of whether the listener is attached to one object or forwarded through a collection.

```javascript
import { ClientListener, CollectionListener, OutputListener } from "gnoblin";

class AuditClient extends ClientListener {
    onTitleChanged(client, title) {
        console.log(client.id, title);
    }
    onFocusChanged(client, focused) {
        console.log(client.title, focused);
    }
    onGeometryChanged(client, geometry) {
        console.log(geometry);
    }
    onClosed(client) {
        console.log("Closed", client.id);
    }
}

gnoblin.clients.addListener(new AuditClient()); // existing and future clients

class TrackCollection extends CollectionListener {
    onAdded(client) {
        console.log("Added", client.appId);
    }
    onRemoved(client) {
        console.log("Removed", client.appId);
    }
}

gnoblin.clients.addListener(new TrackCollection());
```

These are different subscriptions: `ClientListener` observes member events; `CollectionListener` observes membership changes. Registration on a `Collection<T>` must automatically attach to current and future members, and detach when a member leaves the collection. `removeListener` reverses the whole subscription. Events are always ordered per object by native revision/sequence number. An `OutputListener` sees mode, scale, connection and viewport-related changes at the correct object scope.

**Example: only one client**:

```javascript
const firefox = gnoblin.clients.find((c) => c.appId === "firefox");
firefox?.addListener(
    new (class extends ClientListener {
        onTitleChanged(client, title) {
            console.log(title);
        }
    })(),
);
```

**Example: react to new members without double-listening**:

```javascript
class AutoPlace extends CollectionListener {
    onAdded(client) {
        if (client.appId === "org.gnome.Nautilus") client.style.cornerRadius = 6;
    }
}

gnoblin.clients.addListener(new AutoPlace());
```

---

## 5. Live collections, selections and inheritance

```typescript
interface Collection<T> extends Iterable<T> {
    readonly length: number;
    at(index: number): T | undefined;
    get(id: string): T | undefined;
    find(predicate: (item: T) => boolean): T | undefined;
    filter(predicate: (item: T) => boolean): T[]; // snapshot
    map<U>(mapper: (item: T) => U): U[]; // snapshot
    where(query: Query<T>): Collection<T>; // LIVE selection
    addListener(listener: CollectionListener<T> | MemberListener<T>): void;
    removeListener(listener: object): void;
}
```

`Collection<T>` is **not** a mutable JS `Array` even if it has array-like reading and iteration. `add()`/`remove()` are supported only on explicitly mutable collections such as `gnoblin.workspaces` and `layout.bindings`. `gnoblin.clients` is compositor-managed and read-only for membership; `workspace.clients.add(client)` transfers the client to that workspace. Physical `gnoblin.outputs` membership is likewise compositor-managed, but it can accept the explicitly permitted `VirtualOutput` type to create a virtual output (`gnoblin.outputs.add(new VirtualOutput(...))`); this is not a way to fabricate physical connectors. `where()` returns a live filtered collection; `filter()` returns a snapshot array.

```javascript
for (const client of gnoblin.clients) console.log(client.title);
const terminals = gnoblin.clients.filter((c) => c.appId.includes("ghostty"));
const browsers = gnoblin.clients.where({ appId: "firefox" });
const code = gnoblin.workspaces.get("development");
console.log(code.clients.length);
```

Style-bearing client collections have an additional `.style` inheritance object. Their use does **not** mutate each client's explicit override. See §6.

---

## 6. OOP, CSS-like style model (without required stylesheets)

There is no `Client.style` class-wide setting. Only **instances** have `client.style`; scopes such as `gnoblin.clients` and `workspace.clients` expose `.style` as _inherited defaults_. Style inheritance replaces a global `gnoblin.appearance` configuration blob.

**Canonical, concise syntax:**

```javascript
// Global defaults: all current and future clients, unless overridden.
gnoblin.clients.style.cornerRadius = 12;

gnoblin.clients.style.borders = [
    { width: 1, color: "#ffffff40" },
    { width: 2, color: "#7c3aed", offset: 3 },
];

gnoblin.clients.style.shadows = [
    { x: 0, y: 8, blur: 24, color: "#00000066" },
    { x: 0, y: 0, blur: 12, spread: 2, color: "#8b5cf655" },
    { inset: true, x: 0, y: 0, blur: 4, color: "#ffffff22" },
];

gnoblin.clients.style.backdropFilter = [{ blur: 18 }, { saturate: 1.25 }];

gnoblin.clients.style.filter = [{ brightness: 1.05 }, { contrast: 1.1 }];
```

**Workspace overrides:**

```javascript
const development = gnoblin.workspaces.get("development");
development.clients.style.cornerRadius = 4;
development.clients.style.backdropFilter = [];
```

**One client:**

```javascript
const client = gnoblin.clients.focused;
client.style.cornerRadius = 20;
client.style.borders[0].color = "#ff8800";
client.style.shadows[1].blur = 40;
```

**Live application-based style selection, no CSS file:**

```javascript
const browsers = gnoblin.clients.where({ appId: "firefox" });
browsers.style.cornerRadius = 16;
browsers.style.backdropFilter = [];
```

A client leaving a selection loses that selection's defaults and reverts to lower-priority inherited values; its explicit overrides remain. Styles inherit **per property**. Ordered arrays (`borders`, `shadows`, `filter`, `backdropFilter`) replace the inherited list as a whole when assigned, but individual list elements are live typed objects and support mutation.

**Cascade precedence, lowest to highest:** built-in defaults → `gnoblin.clients.style` → `workspace.clients.style` → matching `where()` selections (explicit `priority`, then registration order) → individual `client.style`. `client.computedStyle` is a read-only resolved snapshot. If a client belongs to exactly one primary workspace, there is no ambiguity about which workspace style it inherits. Explicit `unset()`/`reset()` can remove an override and reveal inherited state; `null` is a legitimate value for optional properties, so it should **not** mean “inherit”.

```javascript
client.style.unset("cornerRadius");
const radius = client.computedStyle.cornerRadius;
console.log(radius);
```

### Style classes

```typescript
class Style extends NativeObject {
    opacity: number;
    filter: ObservableCollection<Filter>;
    backdropFilter: ObservableCollection<BackdropFilter>;
}
class ClientStyle extends Style {
    cornerRadius: number | CornerRadii;
    borders: ObservableCollection<Border>;
    shadows: ObservableCollection<Shadow>;
    background: Color | null;
}
class Border extends NativeObject {
    width: number;
    color: Color;
    offset: number;
    inset: boolean;
}
class Shadow extends NativeObject {
    x: number;
    y: number;
    blur: number;
    spread: number;
    color: Color;
    inset: boolean;
}
class BlurFilter extends NativeObject {
    radius: number;
}
class SaturateFilter extends NativeObject {
    amount: number;
}
class BrightnessFilter extends NativeObject {
    amount: number;
}
```

Shorthand JS literals normalise to native style objects, **not** static classes. An equivalent verbose OOP form is available:

```javascript
import { ClientStyle, Border, Shadow, BlurFilter } from "gnoblin";
const style = new ClientStyle({ cornerRadius: 12 });
style.borders.add(new Border({ width: 2, color: "#7c3aed" }));
style.shadows.add(new Shadow({ x: 0, y: 8, blur: 24, color: "#0006" }));
style.backdropFilter.add(new BlurFilter({ radius: 18 }));
gnoblin.clients.style = style;
```

**Paint semantics:** `filter` transforms the client content as an ordered chain. `backdropFilter` samples scene content _behind_ the client, and must not sample security-masked content. `borders` are ordered concentric/offset strokes; multiple layers are supported. `shadows` follow CSS-like drawing order for outside/inset layers. GPU passes have explicit performance caps; invalid style mutations reject with actionable property paths rather than silently dropping effects.

**Different workspace aesthetics**:

```javascript
work.clients.style.shadows = [];
gaming.clients.style.borders = [{ width: 2, color: "#f97316" }];
media.clients.style.filter = [{ saturate: 0.8 }, { contrast: 1.15 }];
```

The engine may optionally support a declarative stylesheet loader someday, but **CSS text is not required for or privileged over OOP styling**. This specification's public contract is the object model and inherited collection styles.

---

## 7. Workspaces are OOP scene objects

`gnoblin.workspaces` is **always present** and is a mutable `Collection<Workspace>`. Each workspace has its own `clients: Collection<Client>`, layout instance, scene transform and lifecycle. A workspace may exist without being visible. **Visibility/activity are not global properties on a workspace**: one workspace may be shown by multiple displays or XR views simultaneously.

```javascript
import { Workspace, FloatingLayout, MasterStackLayout, GridLayout } from "gnoblin";

const desktop = new Workspace({
    id: "desktop",
    name: "Desktop",
    layout: new FloatingLayout({ placement: "smart" }),
});
const development = new Workspace({
    id: "development",
    name: "Development",
    layout: new MasterStackLayout({ ratio: 0.65, gap: 12 }),
});
const web = new Workspace({
    id: "web",
    layout: new GridLayout({ columns: "auto", gap: 8 }),
});

gnoblin.workspaces.add(desktop);
gnoblin.workspaces.add(development);
gnoblin.workspaces.add(web);
```

**Moving a managed toplevel:**

```javascript
const client = gnoblin.input.primary.focusedClient;
if (client) development.clients.add(client); // transfer primary membership
console.log(client?.workspace === development);
```

**Subclass a workspace:**

```javascript
import { CollectionListener, Workspace, MasterStackLayout } from "gnoblin";

class DevelopmentWorkspace extends Workspace {
    constructor() {
        super({
            id: "dev",
            layout: new MasterStackLayout({ ratio: 0.7, gap: 10 }),
        });
        this.clients.addListener(
            new (class extends CollectionListener {
                onAdded(client) {
                    if (client.appId.includes("ghostty")) client.style.cornerRadius = 0;
                }
            })(),
        );
    }
}

gnoblin.workspaces.add(new DevelopmentWorkspace());
```

**Selecting a workspace is a presentation operation, not an activation flag:**

```javascript
const display = gnoblin.input.primary.activeDisplay;
if (display) display.viewport.content = gnoblin.workspaces.get("dev");
```

Workspaces should not be _cloned_ merely because two displays are showing them. Cloning a client is the job of a `View`, which renders a reference to the same underlying source. A client has one primary workspace membership and one Wayland configure size.

---

## 8. Scene, view, cameras and viewport

### Objects

```typescript
class Scene extends NativeObject {
    readonly root: Node;
    metresPerUnit: number;
}
class View extends Node {
    content: Node; // references source without reparenting
    size: Size2D; // local physical/logical panel size (scene units)
    contentFit: "contain" | "cover" | "stretch";
    input: "forward" | "capture" | "none";
}
class Camera2D extends Node {
    position: Vec2;
    zoom: number;
    rotation: number;
}
class Camera3D extends Node {
    position: Vec3;
    rotation: Quaternion | Euler3;
    projection: "perspective" | "orthographic";
    fov: number; // when perspective
    size: number; // when orthographic
    near: number;
    far: number;
}
class Viewport extends NativeObject {
    content: Node;
    camera: Camera2D | Camera3D;
    clearColor: Color;
}
```

**A normal desktop is a 3D scene viewed orthographically:**

```javascript
const viewport = new Viewport({
    content: desktop,
    camera: new Camera2D({ zoom: 1 }),
});
```

**Same workspace through a perspective camera:**

```javascript
const perspective = new Viewport({
    content: desktop,
    camera: new Camera3D({
        projection: "perspective",
        fov: 60,
        position: { x: 0, y: 0, z: 1400 },
    }),
});
```

**An explicit general-purpose orthographic 3D camera:**

```javascript
const camera = new Camera3D({
    projection: "orthographic",
    size: 1800,
    position: { x: 0, y: 0, z: 1500 },
});
```

`Camera2D` is a convenient orthographic camera in pixel-oriented coordinates. `Camera3D` supports perspective **and** orthographic projection; there is no need for separate `CameraOrthographic` and `CameraPerspective` classes. Stereo and tracked-device specifics belong to the optional XR extensions.

**A source `Client` shown in another place as a panel:**

```javascript
const panel = new View({
    content: firefoxClient,
    size: { width: 0.9, height: 0.55 },
    contentFit: "contain",
    input: "forward",
});
spatialRoom.add(panel);
```

`input: "forward"` means scene rays and clicks on the `View` can reach the source client, translated through the view's inverse transform. It does **not** let a different display independently negotiate a different logical client size.

**A scene containing the same source twice:**

```javascript
const overview = new Node({ id: "overview" });
overview.add(new View({ content: desktop, size: { width: 700, height: 400 } }));
overview.add(new View({ content: desktop, size: { width: 350, height: 200 } }));
```

**Safety:** view references cannot form cycles; secure clients must be masked where policy prohibits capture; the lock surface must remain correctly gated across all viewports, virtual displays and XR.

---

## 9. Outputs, output profiles and logical displays

The _physical connector_ and the _presentation_ are separate objects:

- `Output`: discovered connected physical/virtual output; read-only capabilities and actual applied state. A physical `Output` is not manually instantiated.
- `OutputProfile`: desired persistent hardware settings, applies on hotplug. A profile is inert when no matching hardware is present.
- `Display`: **logical presentation** configured from one or several `Output`/`OutputProfile` objects, with **one** `Viewport` and mode `"single"` (default), `"mirror"` or `"span"`.
- `gnoblin.outputs: Collection<Output>` and `.profiles: Collection<OutputProfile>`; `gnoblin.displays: Collection<Display>`.
- `Output` is not a workspace; `Display.viewport.content` chooses which workspace or scene to show.

### Hardware profiles (OOP)

```javascript
import { OutputProfile } from "gnoblin";

const ultrawide = new OutputProfile({
    match: { connector: "DP-1" },
    mode: { width: 3440, height: 1440, refreshRate: 144 },
    scale: 1.25,
    transform: "normal",
    vrr: true,
    position: { x: 0, y: 0 },
});

const side = new OutputProfile({
    match: { connector: "HDMI-A-1" },
    mode: "preferred",
    scale: 1,
    position: { rightOf: ultrawide, align: "top" },
});

gnoblin.outputs.profiles.add(ultrawide);
gnoblin.outputs.profiles.add(side);
```

The profile manager validates and **commits hardware changes atomically**, testing modesets and preserving the last good arrangement. It handles connector arrival/removal, EDID match fallback and no-enabled-output rejection. This is not a piecemeal `output.mode = ...; output.position = ...;` protocol that can strand the desktop mid-update. Hardware profile mutation itself can be batched by the same underlying transaction engine.

**Inspect a physical output:**

```javascript
const dp1 = gnoblin.outputs.get("DP-1");
if (dp1) {
    console.log(dp1.connector, dp1.mode, dp1.scale, dp1.position, dp1.connected);
}
```

**Use EDID identity instead of an unstable connector name:**

```javascript
const laptopPanel = new OutputProfile({
    match: { make: "BOE", model: "Display", serial: "device-specific-serial" },
    mode: "preferred",
    scale: 1.5,
});
```

### A. Independent displays, independent workspaces

```javascript
import { Display, Viewport, Camera2D } from "gnoblin";

const leftDisplay = new Display({
    outputs: [ultrawide],
    viewport: new Viewport({ content: development, camera: new Camera2D() }),
});
const rightDisplay = new Display({
    outputs: [side],
    viewport: new Viewport({ content: web, camera: new Camera2D() }),
});

gnoblin.displays.add(leftDisplay);
gnoblin.displays.add(rightDisplay);
```

`new Display({outputs:[profile]})` accepts `OutputProfile` as a persistent selector; at runtime `display.outputs` resolves to the connected `Collection<Output>`. If no matching output is connected, the configured logical display is parked and its workspace remains intact.

**Switch just the active monitor's workspace:**

```javascript
const display = gnoblin.input.primary.activeDisplay;
if (display) display.viewport.content = gnoblin.workspaces.get("development");
```

### B. Same workspace on two monitors, **not** exact mirroring

```javascript
leftDisplay.viewport.content = development;
rightDisplay.viewport.content = development;
rightDisplay.viewport.camera = new Camera3D({
    projection: "perspective",
    fov: 50,
    position: { x: 0, y: 0, z: 1600 },
});
```

These are _two independent views of the same workspace_. They may use different cameras/zoom/cropping, but the same `Client` instances and a single authoritative layout coordinate system. A Wayland client is not configured at two different logical sizes at once.

### C. Pixel-identical mirror

```javascript
const mirror = new Display({
    outputs: [ultrawide, side],
    mode: "mirror",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
gnoblin.displays.add(mirror);
```

One composed logical frame is sent to both outputs. Matching aspect ratio preserves the picture; differing physical modes may require fitting/letterboxing/scaling. Mirrors do **not** create a second pointer-space rectangle. The current native output engine already has a related mirror concept.

### D. Extended/spanned desktop

```javascript
const span = new Display({
    outputs: [ultrawide, side],
    mode: "span",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
gnoblin.displays.add(span);
```

A _single_ viewport spans the outputs' logical union. Each output shows a clipped piece in its corresponding position. A client may straddle the boundary. Different physical output scales require correct logical-to-buffer transformation. This extends today's output arrangement/rendering and needs explicit new tests.

### E. Multiple sets of workspaces and displays

```javascript
// Each display chooses independently from the same global workspace registry.
const workspaces = [desktop, development, web, gaming];
for (const workspace of workspaces) gnoblin.workspaces.add(workspace);

gnoblin.input.primary.bind("Super+1", () => {
    gnoblin.input.primary.activeDisplay.viewport.content = desktop;
});
gnoblin.input.primary.bind("Super+2", () => {
    gnoblin.input.primary.activeDisplay.viewport.content = development;
});
```

**Canonical geometry rule:** a workspace has a single logical layout area at a time, provided by `workspace.bounds` or by a nominated authority display; other viewports transform that scene. The contract must not implicitly resize the workspace every time any observer resizes. A configurable authority policy (fixed bounds, primary presentation, last-active presentation) can be added, with `fixed` recommended for simultaneously viewed workspaces.

**Display lifecycle/hardware constraints:** an output should belong to exactly one _active_ `Display` at any time. Adding a display that overlaps an already-owned output must atomically replace or reject the old assignment. The user should not have to remove overlapping display objects manually just to select a saved display profile; provide `gnoblin.displays.replace(old, new)` or a grouping/profile switch transaction. Example: `gnoblin.displays.replace(leftDisplay, mirroredDisplay)` (and remove any other conflicting display as one validated batch).

---

## 10. InputContext and devices (not “mouse = seat”)

`InputContext` is the friendly public name for the **logical input group** exported as a Wayland `wl_seat`. A keyboard is **a device**, not a seat. One context can contain several keyboards, mice, touchpads, touchscreens and tablets; it holds unified keyboard/pointer/touch focus. XR can create a separate input context for gaze and tracked controllers.

```typescript
class InputManager {
    readonly contexts: Collection<InputContext>;
    readonly devices: Collection<InputDevice>;
    readonly primary: InputContext;
}
class InputContext extends NativeObject {
    readonly devices: Collection<InputDevice>;
    readonly keyboard: KeyboardState | null;
    readonly pointer: PointerState | null;
    readonly touch: TouchState | null;
    focusedClient: Client | null;
    activeDisplay: Display | null;
    readonly bindings: Collection<Binding>;
    bind(trigger: string, action: Function | Binding): Binding;
}
class InputDevice extends NativeObject {
    name: string;
    enabled: boolean;
    context: InputContext;
    readonly bindings: Collection<Binding>;
    bind(trigger: string, action: Function | Binding): Binding;
}
class Keyboard extends InputDevice {}
class Mouse extends InputDevice {}
class Touchpad extends InputDevice {}
class Touchscreen extends InputDevice {}
class Tablet extends InputDevice {}
```

`gnoblin.input.primary` is the ordinary default context. `gnoblin.clients.focused` is an optional _convenience alias_ for `gnoblin.input.primary.focusedClient`, but focus must be per context internally, not an ambiguous global singleton.

**One method for keyboard, mouse and gestures:**

```javascript
const input = gnoblin.input.primary;
input.bind("Super+Return", () => gnoblin.spawn("ghostty"));
input.bind("Super+Q", () => input.focusedClient?.close());
input.bind("Swipe3Left", () => {
    input.activeDisplay.viewport.content = gnoblin.workspaces.get("web");
});
input.bind("MouseMiddle", (event) => console.log(event.position, event.client));
```

**Bind only a particular physical mouse:**

```javascript
const mouse = gnoblin.input.devices.find((d) => d.name === "Logitech MX Master");
mouse?.bind("Super+MouseMiddle", (event) => console.log("Only this mouse", event));
```

The modifier `Super` in a mouse-specific binding is resolved from the mouse's `InputContext.keyboard`, not from the mouse itself. Reassigning physical devices to contexts must release old pressed state and prevent stuck keys/buttons.

**Device configuration:**

```javascript
import { Mouse, Keyboard, Touchpad } from "gnoblin";
for (const device of gnoblin.input.devices) {
    if (device instanceof Mouse) {
        device.sensitivity = 0.5;
        device.acceleration = "flat";
    }
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
    if (device instanceof Touchpad) {
        device.naturalScroll = true;
        device.tapToClick = true;
    }
}
```

**Independent input contexts:**

```javascript
const secondSeat = new InputContext({ name: "secondary" });
gnoblin.input.contexts.add(secondSeat);
const keyboard = gnoblin.input.devices.find((d) => d.name === "USB Keyboard");
if (keyboard) keyboard.context = secondSeat;
```

Input source and target must be separate. A mouse might target a client on a 2D display while an XR controller ray targets a `View` of the same client. In either case, pointer focus and grabs are scoped to the originating input context.

---

## 11. Binding framework — all behaviours can be written in JavaScript

The trigger matcher and input capture are runtime services. **Specific drag behaviour is JavaScript**, including the built-in library implementations. The binding names intentionally have the `Binding` suffix, so the class's purpose is immediately clear.

```text
Binding
├── KeyBinding
├── PointerBinding
│   └── PointerDragBinding
│       ├── ClientMoveBinding
│       ├── ClientResizeBinding
│       ├── ClientSwapBinding
│       └── SplitResizeBinding
├── GestureBinding
│   └── WorkspaceSwitchBinding
└── (optional XR extension)
    ControllerDragBinding
    ├── ViewMoveBinding
    ├── ViewResizeBinding
    ├── ViewRotateBinding
    └── ViewScaleBinding
```

### Binding and drag lifecycle contract

```typescript
class Binding extends NativeObject {
    id: string;
    trigger: string;
    enabled: boolean;
    priority: number;
}
abstract class DragBinding extends Binding {
    onStart(event: DragStartEvent): object | false;
    onUpdate(event: DragUpdateEvent, state: object): void;
    onEnd(event: DragEndEvent, state: object): void;
    onCancel(event: DragCancelEvent, state: object): void;
}
class PointerDragBinding extends DragBinding {}
```

**Refinement of the earlier `return true` sketch:** `onStart()` should return a fresh **per-gesture state object**, or `false` to decline. This eliminates state races if one binding instance is activated by two independent input contexts/controllers. Each later callback receives its own state. A binding instance is reusable, and the host owns capture/release lifecycle.

The runtime invokes `onStart` after trigger recognition, hit-test and drag threshold; it automatically captures appropriate subsequent input while that gesture is accepted, and releases on end/cancel/crash. The callbacks operate on native-backed objects, but there is **no** `beginNativeGrab()` call and no native move/resize/swap operation required. Host capabilities are limited to safe, generic input and scene mechanisms.

**Common event shape:**

```typescript
interface DragEvent {
    context: InputContext;
    device: InputDevice;
    client?: Client; // if a managed toplevel is hit
    view?: View; // if a scene view is hit
    layout?: Layout;
    position?: Vec2; // stable display/workspace logical coordinates
    localPosition?: Vec2; // target-local coordinates
    ray?: Ray3;
    controller?: XRController; // optional XR module type
    timestamp: number;
    deltaTime: number;
    modifiers: ReadonlySet<string>;
    hit?: HitResult;
}
```

`event.position` is **workspace logical space** for ordinary pointer drags; XR events define `event.ray` and `controller.pose` in a named tracking space. This avoids the silent screen/world coordinate confusion common in input APIs. `deltaTime` is seconds, nonnegative and capped for simulation uses. Input messages are ordered by seat/input context and carry timestamps.

### Triggers are layout constructor options

```javascript
const floating = new FloatingLayout({
    placement: "smart",
    triggers: {
        move: "Super+MouseLeft+Drag",
        resize: "Super+MouseRight+Drag",
    },
});

const tiling = new MasterStackLayout({
    ratio: 0.65,
    triggers: {
        swap: "Super+MouseLeft+Drag",
        resize: "Super+MouseRight+Drag",
    },
});
```

- Omitted trigger or `undefined`: use the built-in default.
- A string: replace the trigger.
- `null`: do not install that default binding.
- `layout.bindings` is a mutable `Collection<Binding>` with stable IDs; `.remove("resize")`, `.get("move")`, `.add(new CustomBinding())`, `.clear()` all work.
- Layout binding matching is restricted to the layout that owns the **targeted client/view**, and scoped to the originating `InputContext`. Global context bindings are fallback unless explicit higher-priority overrides exist. The matcher refuses ambiguous equal-priority bindings with an actionable conflict diagnostic.
- If a layout disappears or its binding is removed during an active grab, that grab gets `onCancel()` once; pointers/buttons cannot become stuck.

**Override only a default trigger:**

```javascript
const floating = new FloatingLayout({
    triggers: { move: "Alt+MouseLeft+Drag", resize: null },
});
```

**Replace a whole default binding:**

```javascript
const floating = new FloatingLayout();
floating.bindings.remove("resize");
floating.bindings.add(
    new ClientResizeBinding({
        id: "resize",
        trigger: "Super+MouseMiddle+Drag",
        edges: "nearest",
    }),
);
```

### Full stock FloatingLayout registration, in JS

```javascript
import { Layout, ClientMoveBinding, ClientResizeBinding } from "gnoblin";

export class FloatingLayout extends Layout {
    constructor({ placement = "smart", triggers = {} } = {}) {
        super();
        this.placement = placement;

        const defaults = {
            move: "Super+MouseLeft+Drag",
            resize: "Super+MouseRight+Drag",
        };
        const actual = { ...defaults, ...triggers };

        if (actual.move !== null) {
            this.bindings.add(
                new ClientMoveBinding({
                    id: "move",
                    trigger: actual.move,
                    snap: 16,
                }),
            );
        }
        if (actual.resize !== null) {
            this.bindings.add(
                new ClientResizeBinding({
                    id: "resize",
                    trigger: actual.resize,
                    edges: "quadrant",
                }),
            );
        }
    }

    onClientAdded(client, { area, clients }) {
        // Only place newly introduced clients. Preserve manual dragging.
        if (client.floatingBounds) return;
        client.bounds = this.findFreeSpace(client, area, clients);
    }

    findFreeSpace(client, area, clients) {
        // Pure smart-placement algorithm supplied in the stock JS library.
        // Search for a non-overlapping rectangle and clamp to work area.
        return {
            x: area.x + 40,
            y: area.y + 40,
            width: Math.min(client.size.width, area.width),
            height: Math.min(client.size.height, area.height),
        };
    }
}
```

The full smart-placement algorithm should be shipped and tested separately rather than hiding it in native Rust. The current Lua floating implementation is a useful behavioural reference for free-space scanning, centring, cascade and minimum-visible-strip constraints.

### ClientMoveBinding — fully JavaScript

```javascript
import { PointerDragBinding } from "gnoblin";

export class ClientMoveBinding extends PointerDragBinding {
    constructor({ id = "move", trigger = "Super+MouseLeft+Drag", snap = 16 } = {}) {
        super({ id, trigger });
        this.snap = snap;
    }

    onStart(event) {
        if (!event.client || event.client.fullscreen) return false;
        return {
            client: event.client,
            startPointer: { ...event.position },
            startBounds: { ...event.client.bounds },
        };
    }

    onUpdate(event, state) {
        const dx = event.position.x - state.startPointer.x;
        const dy = event.position.y - state.startPointer.y;
        const requested = {
            ...state.startBounds,
            x: state.startBounds.x + dx,
            y: state.startBounds.y + dy,
        };
        // A reusable JS helper; not a magical native grab operation.
        state.client.bounds = this.snapBounds(requested, event.layout, this.snap);
    }

    onEnd(_event, state) {
        state.client.floatingBounds = { ...state.client.bounds };
    }

    onCancel(_event, state) {
        state.client.bounds = state.startBounds;
    }

    snapBounds(rect, layout, distance) {
        const area = layout.workspace.bounds;
        const snap = (value, edge) => (Math.abs(value - edge) <= distance ? edge : value);
        return {
            ...rect,
            x: snap(rect.x, area.x),
            y: snap(rect.y, area.y),
        };
    }
}
```

Production snapping would also compare the client's right/bottom edges and other clients, clamp to work-area policy, and respect maximised/fullscreen conversion policy. Crucially, it's all ordinary JS geometry built on existing native property values.

### ClientResizeBinding — quadrant TL/TR/BL/BR

```javascript
import { PointerDragBinding } from "gnoblin";

export class ClientResizeBinding extends PointerDragBinding {
    constructor({ id = "resize", trigger = "Super+MouseRight+Drag", edges = "quadrant" } = {}) {
        super({ id, trigger });
        this.edges = edges;
    }

    onStart(event) {
        if (!event.client || event.client.fullscreen) return false;
        const r = { ...event.client.bounds };
        const where = event.localPosition;
        const corner =
            this.edges === "quadrant"
                ? `${where.y < r.height / 2 ? "top" : "bottom"}-${where.x < r.width / 2 ? "left" : "right"}`
                : this.edges;
        return {
            client: event.client,
            start: r,
            pointer: { ...event.position },
            corner,
        };
    }

    onUpdate(event, state) {
        const dx = event.position.x - state.pointer.x;
        const dy = event.position.y - state.pointer.y;
        const r = { ...state.start };
        if (state.corner.includes("left")) {
            r.x += dx;
            r.width -= dx;
        }
        if (state.corner.includes("right")) {
            r.width += dx;
        }
        if (state.corner.includes("top")) {
            r.y += dy;
            r.height -= dy;
        }
        if (state.corner.includes("bottom")) {
            r.height += dy;
        }

        const min = state.client.minSize ?? { width: 100, height: 80 };
        // Preserve the opposite anchor while clamping sizes.
        if (r.width < min.width) {
            if (state.corner.includes("left")) r.x -= min.width - r.width;
            r.width = min.width;
        }
        if (r.height < min.height) {
            if (state.corner.includes("top")) r.y -= min.height - r.height;
            r.height = min.height;
        }
        state.client.bounds = r;
    }

    onEnd(_event, state) {
        state.client.floatingBounds = { ...state.client.bounds };
    }
    onCancel(_event, state) {
        state.client.bounds = state.start;
    }
}
```

`ClientResizeBinding` is for a **managed flat client** and may update its desired geometry while dragging. In Wayland, the underlying application buffer can lag behind configure requests; the compositor must keep rendering the last committed buffer safely. **XR `ViewResizeBinding` has deliberately different configure timing**, described later.

### ClientSwapBinding — tiling reorder

```javascript
import { PointerDragBinding } from "gnoblin";

export class ClientSwapBinding extends PointerDragBinding {
    constructor({ id = "swap", trigger = "Super+MouseLeft+Drag" } = {}) {
        super({ id, trigger });
    }
    onStart(event) {
        if (!event.client || !event.layout) return false;
        return { dragged: event.client, layout: event.layout, over: null };
    }
    onUpdate(event, state) {
        const hit = event.context.hitTest(event.position);
        state.over = hit?.client?.workspace === state.dragged.workspace ? hit.client : null;
        state.layout.previewSwap = state.over ? { from: state.dragged, to: state.over } : null;
    }
    onEnd(_event, state) {
        state.layout.previewSwap = null;
        if (state.over && state.over !== state.dragged) {
            state.layout.order.swap(state.dragged, state.over);
        }
    }
    onCancel(_event, state) {
        state.layout.previewSwap = null;
    }
}
```

`hitTest()` and `layout.order` are generic native-backed interfaces for scene hit results and ordered layout membership—not a hidden C++ swap algorithm. `previewSwap` can be a typed preview property that the normal transition engine renders as a provisional layout.

### SplitResizeBinding — tiling ratio

```javascript
export class SplitResizeBinding extends PointerDragBinding {
    constructor({ id = "resize", trigger = "Super+MouseRight+Drag" } = {}) {
        super({ id, trigger });
    }
    onStart(event) {
        if (!event.layout || typeof event.layout.ratio !== "number") return false;
        return {
            layout: event.layout,
            startX: event.position.x,
            startRatio: event.layout.ratio,
            width: event.layout.workspace.bounds.width,
        };
    }
    onUpdate(event, state) {
        const dx = event.position.x - state.startX;
        state.layout.ratio = Math.min(0.9, Math.max(0.1, state.startRatio + dx / state.width));
    }
    onCancel(_event, state) {
        state.layout.ratio = state.startRatio;
    }
    onEnd() {}
}
```

For hierarchical tiling, expose typed `Split` objects rather than treating the entire layout as one global `ratio`. A `SplitResizeBinding` can target the split under the pointer and change its specific `ratio`. There is still no requirement for a compiled algorithm.

### Gesture binding: workspace switching with scrubbing

```javascript
import { GestureBinding } from "gnoblin";

class WorkspaceSwitchBinding extends GestureBinding {
    constructor({ trigger = "Swipe3Left" } = {}) {
        super({ trigger });
    }
    onStart(event) {
        const display = event.context.activeDisplay;
        if (!display) return false;
        const next = gnoblin.workspaces.at(2);
        const transition = gnoblin.requestTransition(
            () => {
                display.viewport.content = next;
            },
            { duration: 300, autoplay: false },
        );
        return { transition };
    }
    onUpdate(event, state) {
        state.transition.progress = event.progress;
    }
    onEnd(event, state) {
        if (event.progress >= 0.5) state.transition.play();
        else state.transition.reverse();
    }
    onCancel(_event, state) {
        state.transition.cancel();
    }
}
```

`GestureBinding` has the same per-gesture state semantics. The native input recogniser supplies normalised progress, and the transition engine handles presentation without running arbitrary JS each display frame.

### General binding precedence and policy

On a new input press/drag: security and lock policy → existing capture/grab → explicit device bindings → applicable target layout bindings → logical input-context bindings → forwarding to client. Exact matching and declared priorities resolve conflicts; a more specific chord beats a less specific one. During an accepted drag, the chosen binding remains active until release/cancel even when the pointer crosses clients, outputs or corners. On cancellation (layout removed, target closed, input context gone, script restart), `onCancel` runs once if the JS host is responsive, and native capture is released even if it is not.

---

## 12. Layout system — OOP + pure computation, with stateful hooks where necessary

`Layout` is a normal extensible class with a native-backed identity, workspace reference and `.bindings`. It is **not** a kind of camera. Its responsibility is positioning/managing nodes, not choosing perspective or outputs.

```typescript
class Layout extends NativeObject {
    readonly workspace: Workspace;
    readonly bindings: Collection<Binding>;
    onClientAdded?(client: Client, context: LayoutContext): void;
    onClientRemoved?(client: Client, context: LayoutContext): void;
    onAreaChanged?(oldArea: Rect, newArea: Rect): void;
    compute?(context: LayoutContext): Placement[];
}
interface LayoutContext {
    workspace: Workspace;
    clients: readonly Client[];
    children: readonly Node[];
    area: Rect;
    focused: Client | null;
}
interface Placement {
    node: Node;
    bounds?: Rect;
    transform?: Transform3D;
    visible?: boolean;
    tiled?: boolean;
}
```

**Why not only `arrange()`?** Floating layouts place arrivals once and retain user-adjusted positions. Tiling/grid/spatial layouts compute all placements on structural or parameter changes. Hence: optional lifecycle hooks for one-off effects, optional `compute(context)` for deterministic placement. The host must not invoke `compute` continuously on every rendered frame; it computes desired placements on invalidation and interpolates the _native render scene_ between results.

**Built-in layout options:**

```javascript
new FloatingLayout({ placement: "smart", triggers: { move: "Super+MouseLeft+Drag" } });
new FloatingLayout({ placement: "center" });
new FloatingLayout({ placement: "cascade", cascadeStep: 32 });
new MasterStackLayout({ masterCount: 1, ratio: 0.65, gap: 12, orientation: "left" });
new GridLayout({ columns: "auto", gap: 8 });
new MonocleLayout();
new DwindleLayout({ split: "aspect", ratio: 0.5, gap: 8 });
new ArcLayout({ radius: 1200, spacing: 0.35 });
```

### Complete master/stack `compute()` example

```javascript
import { Layout, ClientSwapBinding, SplitResizeBinding } from "gnoblin";

class MasterStackLayout extends Layout {
    constructor({ ratio = 0.65, gap = 12, triggers = {} } = {}) {
        super();
        this.ratio = ratio;
        this.gap = gap;
        const actual = {
            swap: "Super+MouseLeft+Drag",
            resize: "Super+MouseRight+Drag",
            ...triggers,
        };
        if (actual.swap !== null) this.bindings.add(new ClientSwapBinding({ id: "swap", trigger: actual.swap }));
        if (actual.resize !== null) this.bindings.add(new SplitResizeBinding({ id: "resize", trigger: actual.resize }));
    }

    compute({ clients, area }) {
        if (!clients.length) return [];
        if (clients.length === 1) return [{ node: clients[0], bounds: area, tiled: true }];
        const masterW = (area.width - this.gap) * this.ratio;
        const sideW = area.width - this.gap - masterW;
        const stack = clients.slice(1);
        const h = (area.height - (stack.length - 1) * this.gap) / stack.length;
        return [
            { node: clients[0], tiled: true, bounds: { x: area.x, y: area.y, width: masterW, height: area.height } },
            ...stack.map((node, i) => ({
                node,
                tiled: true,
                bounds: { x: area.x + masterW + this.gap, y: area.y + i * (h + this.gap), width: sideW, height: h },
            })),
        ];
    }
}
```

### Simple grid `compute()`

```javascript
class GridLayout extends Layout {
    constructor({ columns = "auto", gap = 8 } = {}) {
        super();
        this.columns = columns;
        this.gap = gap;
    }
    compute({ clients, area }) {
        if (!clients.length) return [];
        const cols = this.columns === "auto" ? Math.ceil(Math.sqrt(clients.length)) : this.columns;
        const rows = Math.ceil(clients.length / cols);
        const w = (area.width - (cols - 1) * this.gap) / cols;
        const h = (area.height - (rows - 1) * this.gap) / rows;
        return clients.map((node, i) => ({
            node,
            tiled: true,
            bounds: {
                x: area.x + (i % cols) * (w + this.gap),
                y: area.y + Math.floor(i / cols) * (h + this.gap),
                width: w,
                height: h,
            },
        }));
    }
}
```

### Spatial arc `compute()` (not XR-specific)

```javascript
class ArcLayout extends Layout {
    constructor({ radius = 1200, spacing = 0.35 } = {}) {
        super();
        this.radius = radius;
        this.spacing = spacing;
    }
    compute({ children, focused }) {
        const selected = Math.max(
            0,
            children.findIndex((n) => n === focused || n.content === focused),
        );
        return children.map((node, i) => {
            const angle = (i - selected) * this.spacing;
            return {
                node,
                transform: {
                    position: { x: Math.sin(angle) * this.radius, y: 0, z: -Math.cos(angle) * this.radius },
                    rotation: { x: 0, y: -angle, z: 0 },
                    scale: 1,
                },
            };
        });
    }
}
```

Spatial layouts produce native node poses. They don't fake 3D with opacity/scale alone. You can present the same `ArcLayout` through `Camera3D` on a monitor or through XR extension cameras if the spatial units are configured accordingly.

### Moving between layouts

```javascript
const flat = new MasterStackLayout({ ratio: 0.65 });
const spatial = new ArcLayout({ radius: 1200 });
const camera2D = new Camera2D();
const camera3D = new Camera3D({ projection: "perspective", fov: 60 });

gnoblin.requestTransition(
    () => {
        workspace.layout = spatial;
        display.viewport.camera = camera3D;
    },
    { duration: 750, easing: "ease-in-out" },
);
```

Layouts are normal OOP objects, but **the host owns the timing and application of their results**, not the layout's JS code. A failed compute leaves the last valid scene placement intact.

---

## 13. XR as an optional native `.so` extension

**Public service:** `gnoblin.extensions.xr` (nullable/absent when not installed).  
**Optional JS exports:** `gnoblin/extensions/xr`.  
**Native implementation:** a versioned `.so`, dynamically loaded by an extension host. The host connects to Gnoblin's existing typed object and view interfaces. The compositor **does not require** any OpenXR library at startup. Optional native module loading should not happen directly inside the compositor process by default: a crashing or blocking OpenXR library must not freeze the desktop.

The current repository implements much of XR as an optional external `gnoblin-xr` binary. The `.so` extension packaging and JavaScript API below are **future design targets**, not the current installation mechanism.

### XR class hierarchy

```text
Core Camera3D ────────────> XR.Camera3D / XRCamera3D
Core Viewport ────────────> XR.Viewport / XRViewport
Core InputContext ───────> session.input (ordinary InputContext, XR device sources)
Core ControllerDragBinding?  (NO — this base is provided by XR extension)
XR.ControllerDragBinding
  ├── XR.ViewMoveBinding
  ├── XR.ViewResizeBinding
  ├── XR.ViewRotateBinding
  └── XR.ViewScaleBinding
XR.Session
  ├── camera/head tracking sources
  ├── controller collection and input context
  ├── XRViewport / composition policy
  └── runtime lifecycle and layer budget
```

The **core** `Camera3D` has no IPD/head tracking/OpenXR runtime-specific properties. `XRCamera3D` adds reference-space tracking and stereo per-eye projections. The core `Viewport` has no headset layer allocation. `XRViewport` adds OpenXR-specific composition policy. This keeps the ordinary compositor API clean.

### Access extension and set up a session

```javascript
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    const { Camera3D: XRCamera3D, Viewport: XRViewport } = xr;
    const headCamera = new XRCamera3D({
        tracking: xr.session.head,
        near: 0.05,
        far: 100,
    });
    xr.session.viewport = new XRViewport({
        content: spatialRoom,
        camera: headCamera,
        composition: "auto",
    });
}
```

When packaged as an ES module, named imports would also be available:

```javascript
import { XRCamera3D, XRViewport, ViewMoveBinding } from "gnoblin/extensions/xr";
```

If the XR extension is absent, **static importing its module fails clearly at load time**. To make XR optional in an otherwise identical config, use a guarded `gnoblin.extensions.xr` access or dynamic `import()` after checking capability.

### What does the XR session own?

```typescript
class XRExtension {
    readonly available: boolean;
    readonly session: XRSession | null;
    readonly sessions: Collection<XRSession>;
    Camera3D: typeof XRCamera3D;
    Viewport: typeof XRViewport;
    SpatialLayout: typeof XRSpatialLayout;
    ViewMoveBinding: typeof XRViewMoveBinding;
    ViewResizeBinding: typeof XRViewResizeBinding;
}
class XRSession extends NativeObject {
    readonly connected: boolean;
    readonly tracking: TrackingState;
    readonly head: TrackedPose;
    readonly controllers: Collection<XRController>;
    readonly input: InputContext; // logical Wayland seat, not a mouse
    readonly layerBudget: number;
    viewport: XRViewport;
}
class XRCamera3D extends Camera3D {
    tracking: TrackedPose;
    referenceSpace: "local" | "stage" | "view";
}
class XRViewport extends Viewport {
    composition: "auto" | "layers" | "projection";
}
```

The extension exposes a logical `InputContext` for gaze/controller rays, rather than a separate incompatible input system. The host sends high-quality head poses through Gnoblin's dedicated narrow native camera-update path; **JavaScript never drives the tracking or headset swapchain loop**.

### XR scene and view creation

A source client remains in its original workspace. The XR room owns `View` nodes referencing it. Thus manipulating a panel normally changes the panel—not the monitor's source client transform.

```javascript
import { Node, View, CollectionListener } from "gnoblin";
const xr = gnoblin.extensions.xr;

const room = new Node({ id: "xr-room" });
room.metresPerUnit = 1;

const panels = new Map();
function addPanel(client) {
    if (client.secure || panels.has(client)) return;
    const ratio = client.committedSize.width / client.committedSize.height;
    const panel = new View({
        content: client,
        size: { width: 0.8, height: 0.8 / ratio }, // metres
        contentFit: "contain",
        input: "forward",
    });
    panels.set(client, panel);
    room.add(panel);
}
function removePanel(client) {
    panels.get(client)?.destroy();
    panels.delete(client);
}

for (const client of gnoblin.clients) addPanel(client);
gnoblin.clients.addListener(
    new (class extends CollectionListener {
        onAdded(client) {
            addPanel(client);
        }
        onRemoved(client) {
            removePanel(client);
        }
    })(),
);

gnoblin.scene.root.add(room);
```

The standard XR room arrangement can use a **JS `SpatialLayout`**, not a native OpenXR layout engine:

```javascript
const spatial = new xr.SpatialLayout({
    radius: 1.6, // metres
    spacing: 0.35, // radians
    triggers: {
        move: "Grip+Drag",
        resize: "Grip+Trigger+Drag",
        rotate: "Trigger+Thumbstick+Drag",
    },
});
room.layout = spatial;
```

That class constructs its own `XR.ViewMoveBinding`, `XR.ViewResizeBinding` etc. The optional XR extension supplies tracking and input types; ordinary JS computes scene behaviour.

### Controller inputs and trigger grammar

XR input profiles are mapped by the native `.so` to stable logical controls: `Aim` (ray), `Trigger`, `Grip`, `Thumbstick`, `Primary`, `Secondary`, and optional `Left.`/`Right.` prefixes. The mapping is separate from the JS bindings, so the same script works across controllers. `Trigger` by itself continues to send ordinary pointer selection to applications; `Grip+Drag` may be intercepted by the layout for panel manipulation. Triggers can be overridden or disabled exactly as they are in `FloatingLayout`.

```javascript
spatial.bindings.get("move").trigger = "Right.Grip+Drag";
spatial.bindings.remove("rotate");
spatial.bindings.add(
    new xr.ViewMoveBinding({
        id: "move",
        trigger: "Trigger+Grip+Drag",
        tracking: "ray",
        rotation: "keep",
    }),
);
```

### XR ViewMoveBinding — pure JS 6-DoF grab

```javascript
class ViewMoveBinding extends xr.ControllerDragBinding {
    constructor({ id = "move", trigger = "Grip+Drag", tracking = "controller", rotation = "follow" } = {}) {
        super({ id, trigger });
        this.tracking = tracking;
        this.rotation = rotation;
    }

    onStart(event) {
        if (!event.view || !event.controller?.tracked) return false;
        return {
            view: event.view,
            startView: event.view.transform.clone(),
            startController: event.controller.pose.clone(),
        };
    }

    onUpdate(event, state) {
        const current = event.controller.pose;
        if (this.tracking === "controller") {
            const delta = current.multiply(state.startController.inverse());
            const target = delta.multiply(state.startView);
            if (this.rotation !== "follow") target.rotation = state.startView.rotation;
            state.view.transform = target;
        } else {
            const ray = event.ray;
            const distance = state.startView.position.distanceTo(state.startController.position);
            state.view.position = ray.at(distance); // keep initial distance
        }
    }

    onEnd() {} // keep the new spatial pose
    onCancel(_event, state) {
        state.view.transform = state.startView;
    }
}
```

This is genuine JS interaction logic, not a call to a built-in `xr-move-grab`. `Pose3D.multiply/inverse`, `Ray3.at`, world/local transform conversion and hit-testing are generic maths primitives accelerated or hosted by native Rust as needed. The example assumes controller and panel transforms share one parent/tracking space; in general use explicit `worldToLocal()` and `localToWorld()` conversions.

### Ray-dragging a distant panel

```javascript
const binding = new xr.ViewMoveBinding({
    trigger: "Grip+Drag",
    tracking: "ray",
    distance: "preserve",
    rotation: "keep",
});
spatial.bindings.add(binding);
```

On start, record the ray hit point and initial distance. On update, intersect the ray with the chosen movement plane or use `ray.at(distance)`, preserving the initial grab offset. The JS class can choose a world-fixed plane, a head-facing plane, depth-free movement or collision/snapping constraints via arguments. The XR runtime supplies the fresh ray; JS chooses the policy.

### XR ViewResizeBinding — live View preview, Client resize on release

**This is a committed design requirement:** resizing a spatial panel modifies `View.size` continuously during the drag; it **does not** repeatedly send Wayland configure requests. On release, JS asks for a new `Client.size` when the aspect ratio changed or the existing buffer density is insufficient for the enlarged panel. Gnoblin maintains the physical panel size and displays the last committed buffer with the selected `contentFit` policy until the new client buffer arrives.

```javascript
class ViewResizeBinding extends xr.ControllerDragBinding {
    constructor({
        id = "resize",
        trigger = "Grip+Trigger+Drag",
        edges = "quadrant",
        pixelsPerMetre = 1200,
        minWidth = 0.25,
        minHeight = 0.15,
    } = {}) {
        super({ id, trigger });
        Object.assign(this, { edges, pixelsPerMetre, minWidth, minHeight });
    }

    onStart(event) {
        if (!event.view || !event.ray) return false;
        const view = event.view;
        const transform = view.worldTransform.clone();
        const hit = transform.plane().intersect(event.ray);
        if (!hit) return false;
        const local = transform.inverse().transformPoint(hit);
        const size = { ...view.size };
        const corner =
            this.edges === "quadrant"
                ? `${local.y < size.height / 2 ? "top" : "bottom"}-${local.x < size.width / 2 ? "left" : "right"}`
                : this.edges;

        return {
            view,
            originalSize: size,
            originalTransform: view.transform.clone(),
            worldPlane: transform.plane(),
            originalInverse: transform.inverse(),
            firstPoint: local,
            corner,
        };
    }

    onUpdate(event, state) {
        const hit = state.worldPlane.intersect(event.ray);
        if (!hit) return;
        const p = state.originalInverse.transformPoint(hit);
        const dx = p.x - state.firstPoint.x;
        const dy = p.y - state.firstPoint.y;

        const rect = { x: 0, y: 0, width: state.originalSize.width, height: state.originalSize.height };
        if (state.corner.includes("left")) {
            rect.x += dx;
            rect.width -= dx;
        }
        if (state.corner.includes("right")) rect.width += dx;
        if (state.corner.includes("top")) {
            rect.y += dy;
            rect.height -= dy;
        }
        if (state.corner.includes("bottom")) rect.height += dy;

        if (rect.width < this.minWidth) {
            if (state.corner.includes("left")) rect.x -= this.minWidth - rect.width;
            rect.width = this.minWidth;
        }
        if (rect.height < this.minHeight) {
            if (state.corner.includes("top")) rect.y -= this.minHeight - rect.height;
            rect.height = this.minHeight;
        }

        // Keep the opposite corner anchored. Translate in the view's
        // original local plane; the host handles world/parent transforms.
        state.view.size = { width: rect.width, height: rect.height };
        state.view.transform = state.originalTransform.multiply(Transform3D.translation(rect.x, rect.y, 0));
    }

    onEnd(_event, state) {
        const view = state.view;
        const client = view.content;
        if (!(client instanceof Client)) return;

        const committed = client.committedSize;
        const desired = {
            width: Math.round(view.size.width * this.pixelsPerMetre),
            height: Math.round(view.size.height * this.pixelsPerMetre),
        };
        const oldRatio = committed.width / committed.height;
        const newRatio = desired.width / desired.height;
        const aspectChanged = Math.abs(newRatio / oldRatio - 1) > 0.02;
        const tooLowResolution = committed.width < desired.width * 0.85 || committed.height < desired.height * 0.85;

        if (aspectChanged || tooLowResolution) {
            client.size = desired; // ONE new desired Wayland toplevel size on release
        }
    }

    onCancel(_event, state) {
        state.view.size = state.originalSize;
        state.view.transform = state.originalTransform;
    }
}
```

The example assumes `Client`, `Transform3D` and relevant math types were imported, and a local panel coordinate system with x to the right and y down in **metres**. Unlike the earlier prototype, it computes intersections against the **original** panel plane, preventing feedback drift as the panel size changes. Production implementation must account for client size hints, maximum viewport density, aspect correction while waiting, XR layer availability, and secure-content restrictions. If the user resizes _smaller_ without aspect change, it may keep the high-resolution buffer until another policy chooses to downsize.

**Shared client size is unavoidable:** if the source Firefox client is also displayed on a monitor, its logical size changes there when the XR resize commits. Other views of the same client can independently scale/pad/crop the new buffer, but cannot each demand a different actual Wayland toplevel size. This should be visible in the XR inspector and documented, not hidden.

### XR view rotation and scaling (small, fully JS)

```javascript
class ViewRotateBinding extends xr.ControllerDragBinding {
    constructor({ trigger = "Trigger+Thumbstick+Drag" } = {}) {
        super({ id: "rotate", trigger });
    }
    onStart(event) {
        if (!event.view) return false;
        return {
            view: event.view,
            original: event.view.rotation.clone(),
            controller: event.controller.pose.rotation.clone(),
        };
    }
    onUpdate(event, s) {
        const delta = event.controller.pose.rotation.multiply(s.controller.inverse());
        s.view.rotation = delta.multiply(s.original);
    }
    onEnd() {}
    onCancel(_event, s) {
        s.view.rotation = s.original;
    }
}

class ViewScaleBinding extends xr.ControllerDragBinding {
    constructor({ trigger = "Grip+Thumbstick+Drag" } = {}) {
        super({ id: "scale", trigger });
    }
    onStart(e) {
        if (!e.view) return false;
        return { view: e.view, scale: e.view.scale, start: e.controller.pose.position };
    }
    onUpdate(e, s) {
        const delta = e.controller.pose.position.distanceTo(s.start);
        s.view.scale = Math.max(0.2, Math.min(3, s.scale + delta));
    }
    onEnd() {}
    onCancel(_e, s) {
        s.view.scale = s.scale;
    }
}
```

This demonstrates how little XR-specific input policy actually needs to be hard-coded in Rust. The input subsystem supplies poses and rays; JS manipulates general `Node` and `View` state.

### User-defined throw / inertia interaction

```javascript
class ThrowViewBinding extends xr.ControllerDragBinding {
    constructor() {
        super({ id: "throw", trigger: "Grip+Drag" });
    }
    onStart(e) {
        if (!e.view) return false;
        return {
            view: e.view,
            last: e.controller.pose.position,
            velocity: Vector3.zero(),
            original: e.view.transform.clone(),
        };
    }
    onUpdate(e, s) {
        const next = e.controller.pose.position;
        const delta = next.subtract(s.last);
        s.view.position = s.view.position.add(delta);
        s.velocity = delta.scale(1 / Math.max(1e-3, e.deltaTime));
        s.last = next;
    }
    onEnd(_e, s) {
        // User-defined original physics stepper, optionally native-accelerated.
        runThrowSimulation(s.view, s.velocity);
    }
    onCancel(_e, s) {
        s.view.transform = s.original;
    }
}
```

`runThrowSimulation` is deliberately an **author-defined function**, not a hidden magical API; it can use JS scheduling or a separately installed physics extension. A native WASM/physics implementation is an optimisation, not a prerequisite for writing the binding.

### XR runtime implementation boundary

The `.so` extension connects to OpenXR, creates session/swapchains, translates runtime controller profiles into logical device actions, supplies tracked poses, and consumes compositor-rendered view/panel buffers. Gnoblin (the compositor and JS shell) retains ownership of placement, decoration, drawing, click routing and ray hit-tests. The extension does **not** choose its own layout or redraw clients behind Gnoblin's back.

**Composition modes:** `"layers"` = planar quad layers; `"projection"` = depth-tested stereoscopic scene render; `"auto"` = choose based on scene shape and layer budget. The extension reports layer budget, but planner policy remains visible/inspectable through the object graph. Head-locked UI is a viewer-attached virtual output so Bingux can present ordinary layer-shell content; Bingux itself is not an XR renderer dependency.

**Hardware honesty:** as of the branch's 2026-11-13 XR docs, real-headset output isn't yet proven; testing covers simulated/null OpenXR paths, synthetic controllers and headless scenes. OpenXR-native controller drags, live physical resizing and this JS class interface are specification targets.

---

## 14. Universal animation / `gnoblin.requestTransition()`

Animation is part of **state mutation**, not a separate graphical afterthought. The most common operation is direct assignment. Any observable state change may be grouped into one coordinated transition without declaring frame-by-frame keyframes.

```javascript
client.opacity = 0.85; // immediate
client.style.cornerRadius = 16; // immediate

gnoblin.requestTransition(
    () => {
        client.opacity = 0.85;
        client.style.cornerRadius = 16;
    },
    { duration: 250, easing: "ease-out" },
);
```

The callback executes **once, synchronously, now**. It is a staging block for tracked property writes. The host validates the entire proposed state, computes before/after scene descriptions and sends **one** compositor transaction. The native tween/transition engine displays the intermediate states. The callback is **not called every display frame**. Side-effectful operations such as `gnoblin.spawn`, HTTP requests or arbitrary filesystem writes are **not valid inside a transition staging callback**; `await` is not supported inside it.

```typescript
interface TransitionOptions {
    duration?: number; // milliseconds (the JS API uses ms consistently)
    delay?: number; // milliseconds
    easing?: string | Easing;
    autoplay?: boolean; // true by default
}
class Transition extends NativeObject {
    progress: number; // 0..1, writable when manually controlled
    readonly running: boolean;
    readonly duration: number;
    readonly finished: Promise<void>;
    play(): void;
    pause(): void;
    reverse(): void;
    finish(): void;
    cancel(): void;
}
```

**Examples:**

```javascript
// Focus dimming, triggered by typed listener.
class FadeOnFocus extends ClientListener {
    onFocusChanged(client, focused) {
        gnoblin.requestTransition(
            () => {
                client.style.opacity = focused ? 1 : 0.85;
            },
            { duration: 160, easing: "ease-out" },
        );
    }
}
gnoblin.clients.addListener(new FadeOnFocus());
```

```javascript
// Animate multiple styles across all clients inheriting collection defaults.
gnoblin.requestTransition(
    () => {
        gnoblin.clients.style.cornerRadius = 24;
        gnoblin.clients.style.shadows[0].blur = 48;
        gnoblin.clients.style.borders[1].width = 4;
    },
    { duration: 400 },
);
```

```javascript
// Animate a custom JS layout field (intercepted by host runtime).
gnoblin.requestTransition(
    () => {
        arcLayout.radius = 1600;
        arcLayout.spacing = 0.55;
    },
    { duration: 700, easing: "ease-in-out" },
);
```

```javascript
// Animate viewport camera pose, without changing layout.
gnoblin.requestTransition(
    () => {
        camera3D.position = { x: 0, y: 100, z: 1600 };
        camera3D.fov = 80;
    },
    { duration: 500, easing: "ease-out" },
);
```

```javascript
// Animate workspace selection and visual effects together.
gnoblin.requestTransition(
    () => {
        display.viewport.content = development;
        gnoblin.clients.style.backdropFilter = [{ blur: 12 }];
    },
    { duration: 350 },
);
```

```javascript
// Animate a spatial View, not the underlying client.
gnoblin.requestTransition(
    () => {
        panel.position = { x: 0.5, y: 0.2, z: -1.4 };
        panel.rotation = { x: 0, y: 0.3, z: 0 };
        panel.size = { width: 1.2, height: 0.75 };
    },
    { duration: 550, easing: "ease-out" },
);
```

### 2D → 3D camera and layout transition

```javascript
const flat = new MasterStackLayout({ ratio: 0.65 });
const spatial = new ArcLayout({ radius: 1200, spacing: 0.35 });
const camera2D = new Camera2D();
const camera3D = new Camera3D({
    projection: "perspective",
    fov: 65,
    position: { x: 0, y: 0, z: 1600 },
});

let in3D = false;
gnoblin.input.primary.bind("Super+Tab", () => {
    in3D = !in3D;
    gnoblin.requestTransition(
        () => {
            workspace.layout = in3D ? spatial : flat;
            display.viewport.camera = in3D ? camera3D : camera2D;
            gnoblin.clients.style.cornerRadius = in3D ? 24 : 10;
        },
        { duration: 800, easing: "ease-in-out" },
    );
});
```

**Internally:** changing a layout reference is a **structural** change. The host computes the current and target placements, pairs nodes by stable ID, and interpolates their geometry/transform tracks. Changing `Camera2D` to `Camera3D` is also structural: the engine creates a meaningful projection interpolation, blending 2D/3D camera pose, viewing scale and frustum around a chosen visual anchor. **Do not linearly interpolate arbitrary projection matrices** or pretend JavaScript objects themselves are vectors. The renderer and hit-testing system use the same intermediate transforms, so cursor selection matches the drawn result.

For a changed source workspace (`display.viewport.content`), the transition stages both scenes and crossfades/slides with explicit input ownership timing. Structural transitions have typed adapters; unsupported object replacements happen discretely at a specified point rather than silently trying to interpolate nonsensical object identities.

### Interactive transitions / gesture scrubbing

```javascript
const transition = gnoblin.requestTransition(
    () => {
        workspace.layout = spatial;
        display.viewport.camera = camera3D;
    },
    { duration: 800, easing: "ease-in-out", autoplay: false },
);

transition.progress = 0.4; // preview an intermediate rendered state
transition.play(); // animate toward the destination
await transition.finished;
```

**Transaction semantics:** a noninteractive transition commits target logical state when playback is initiated; native _rendered_ values interpolate to it. For manually controlled transitions (`autoplay:false`), stage the target without immediately changing the committed logical state; `progress` previews intermediate visuals; completing at `1` commits the target, reversing to `0` keeps/restores the start. While scrubbing, input hit-testing uses the interpolated scene. The host needs to version/lock the affected state against conflicting external mutations, or retarget safely when they occur. `cancel()` restores the authoritative state according to whether the target was committed, releases tracks and settles its promise with a cancellation reason.

### Optional per-object `.set()` shorthand

```javascript
client.set(
    {
        opacity: 0.92,
        transform: { rotation: { x: 0, y: 0.12, z: 0 } },
    },
    { duration: 250, easing: "ease-out" },
);
```

This is equivalent to one `requestTransition()` affecting a single object's native state. It's a convenience, not a second animation subsystem. Direct properties remain the primary API.

### Interpolable values and semantics

| Value                                                    | Behaviour                                                                                          |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Numbers, lengths, opacity, layout ratios                 | Numeric interpolation, finite/valid range checked                                                  |
| 2D/3D vectors                                            | Per-axis vector interpolation                                                                      |
| Quaternions / 3D orientation                             | Spherical or equivalent shortest-path rotation interpolation                                       |
| Colours                                                  | Documented colour space (prefer perceptual for authored colour transitions)                        |
| Shadows/borders/filters                                  | Match layers by stable identity where possible, else list index; add/remove via fade when feasible |
| Layout instances                                         | Interpolate resulting placements for stable node IDs                                               |
| Camera2D↔Camera3D                                        | Dedicated projection/pose blend with continuity anchor                                             |
| Scene reparenting / View insertion                       | Structural transition adapter, respecting ownership and cycles                                     |
| Discrete state: mode setting, security, process creation | Immediate validated commit / explicit transition point, never fake interpolation                   |
| Custom JS plain numeric fields                           | Host-intercepted assignment, invalidation and layout endpoint recomputation                        |
| Arbitrary JS objects/functions                           | Not inherently interpolable; require an adapter or discrete change                                 |

**Property ownership:** if two animations write the same component of an object, the newest transition supersedes the previous track in that component while independent tracks continue. The current native renderer already has a version of this rule. Transitions should be retargetable without visible jumps. `finished` resolves/rejects once. Scene/viewport references must remain alive for the transition's duration or tracks cancel cleanly.

**Reduced motion:** a per-input-context or session preference may clamp duration to zero, while preserving logically equivalent end state and lifecycle notifications.

---

## 15. Binding animations to state without polling

For more advanced effects, a native dependency graph allows derived values to follow other observable properties. An explicit binding API is acceptable here because it describes a **persistent relation**, not a one-time setting.

```javascript
client.bind("opacity", () => {
    const distance = client.position.distanceTo(camera3D.position);
    return Math.max(0.25, 1 - distance / 2500);
});
```

`bind()` is overloaded by target type: on `InputContext` or `InputDevice`, it binds a trigger; on `NativeObject`, it establishes a tracked property dependency. To eliminate confusion, an implementation can instead expose `client.properties.bind("opacity", ...)` as the canonical property form, while reserving `.bind()` for input triggers. **The recommended contract is `client.properties.bind` for reactive values and `input.bind` for input triggers**, with no ambiguous runtime overload.

```javascript
client.properties.bind("opacity", () => {
    const distance = client.position.distanceTo(camera3D.position);
    return Math.max(0.25, 1 - distance / 2500);
});
```

The callback runs when dependencies change, not on a fixed JavaScript frame timer. Native, known calculations may be compiled/represented as graph operators; arbitrary JS functions remain host-evaluated under budgets. Cycle detection must report specific dependencies and avoid an endless recompute loop.

**Examples:**

```javascript
// Bind a vignette strength to the camera's zoom.
background.properties.bind("vignette", () => Math.min(1, camera2D.zoom / 3));

// Bind background blur to how many clients are in the workspace.
development.properties.bind("blurRadius", () => (development.clients.length > 4 ? 12 : 0));
```

Property bindings are not input bindings, and they must have clear lifetime and unsubscribe semantics. The host should cancel them when the owning node is destroyed or hot reload replaces their generation.

---

## 16. Native-backed JS objects and “invisible” change tracking

**No user-written `Proxy`, `Reflect`, decorators, `static properties` or registration tables.** Developers write natural classes:

```javascript
class SpiralLayout extends Layout {
    constructor({ radius = 900, spacing = 0.35 } = {}) {
        super();
        this.radius = radius;
        this.spacing = spacing;
    }
    compute({ children }) {
        return children.map((node, i) => ({
            node,
            transform: {
                position: {
                    x: Math.sin(i * this.spacing) * this.radius,
                    y: 0,
                    z: -Math.cos(i * this.spacing) * this.radius,
                },
            },
        }));
    }
}

const spiral = new SpiralLayout();
workspace.layout = spiral;
gnoblin.requestTransition(
    () => {
        spiral.radius = 1300;
    },
    { duration: 500 },
);
```

**Native-backed properties:** `client.style.cornerRadius`, `view.size`, `camera.position`, `workspace.layout`, `display.viewport` and typed layer objects are backed by native object IDs and property IDs. The host binding layer validates and records revisions/transactions in Rust. This is straightforward.

**Ordinary user-defined JS fields:** `spiral.radius` is **not inherently visible to Rust**. The JS engine normally stores an ordinary own property without calling a Rust setter. To support seamless observation, Gnoblin must build **an engine-level interception bridge** for objects extending observable framework bases (`Layout`, `Binding`, etc.). Implementation choices include invisible host object wrappers, engine-specific property hooks or an internal proxy established by the library. That proxy, if used, is **implementation machinery**, never code the user writes. It must preserve ES class semantics, identity, reflection, method dispatch and private-field behaviour as far as supported.

**Explicit limitation:** arbitrary writes to an unwrapped plain object (e.g. `let x = { nested: { value: 1 } }; x.nested.value++`) cannot be observed solely from the Rust scene store. For nested tracked values, Gnoblin must provide native-backed typed objects or wrap observed child objects as well. If a value cannot be tracked, the host should say so rather than promising magic it cannot implement.

**Suggested implementation:** native objects use generated getter/setter host bindings and store revisions in Rust; user-defined subclasses of the specific observable bases use transparent script-runtime interception and send property-change records to the same Rust invalidation system. `compute` invalidation can be conservative (any observed layout property write marks its layout dirty) initially, then dependency-specific later. This avoids requiring the user to enumerate their fields.

**Transactional property write pipeline:**

```text
JS assignment on native/observable object
 → resolve ObjectId / key and typed value
 → collect in runtime transaction (or staging requestTransition)
 → validate against immutable object schema and scene ownership
 → apply atomically at compositor frame boundary
 → update property revision / invalidate layout and style dependencies
 → evaluate native tracks and send necessary Wayland configure requests
 → render from the same transform state that input hit-testing uses
```

Existing `gnoblin-render` scene transactions and tween tracking provide part of this; the observable custom-JS bridge, class registry and universal transition planner are new engineering work.

### Property limits and failure safety

A failed write names its target and field: e.g. `Client#42.style.borders[2].width: must be finite and ≥ 0`. No NaNs, infinity, invalid scales, cycles or unlimited effect-pass chains reach the renderer. The underlying compositor remains responsive even when a user script times out, loops, crashes or allocates too much memory. A rejected transition leaves the last valid state in place.

---

## 17. Extension `.so` lifecycle and ABI

A native extension is not a magic patch applied to the compositor process. It is a **versioned plugin** loaded by an isolated extension host with a defined object, service and event API. A library can provide classes to the JS runtime, input devices, hardware integrations, native scene resources, specialised effect/interpolation adapters or optional physics/network capabilities. XR is the first flagship extension.

**Proposed packaging:**

```text
/usr/lib/gnoblin/extensions/xr/
├── manifest.json
├── libgnoblin_xr.so
└── js/index.mjs     # host-generated/bundled JS class bindings, optional
```

**Example manifest** (conceptual):

```json
{
    "id": "xr",
    "version": "1.0.0",
    "abi": 1,
    "library": "libgnoblin_xr.so",
    "module": "gnoblin/extensions/xr",
    "capabilities": ["view-consumer", "input-source", "tracked-pose", "xr-session"],
    "load": "optional"
}
```

**Native registration** (C-ABI style pseudocode; exact struct layout must be frozen before implementation):

```c
GNOBLIN_EXPORT int gnoblin_extension_init(
    const GnoblinHostAPI *host,
    GnoblinExtensionRegistration *registration
) {
    if (host->abi_version != GNOBLIN_EXTENSION_ABI_1) return GNOBLIN_ABI_MISMATCH;
    registration->id = "xr";
    registration->create_service = xr_create_service;
    registration->destroy_service = xr_destroy_service;
    registration->register_types = xr_register_types;
    return GNOBLIN_OK;
}
```

Rules: (1) ABI version handshake; (2) capability-grant enforcement; (3) extension-host process isolation by default; (4) explicit lifecycle (load/ready/unload/error/restart); (5) no direct unrestricted writable pointer into compositor scene storage; (6) extension-generated JS types inherit ordinary core types and are only importable when extension installed; (7) audit view capture and device injection; (8) restore native resources and release logical input contexts on unload. An optional **explicitly trusted** in-process tier could be added in future for narrow high-performance cases, but is not part of the safe default contract.

**Extension registry:**

```javascript
const xr = gnoblin.extensions.xr; // null / undefined if not loaded
for (const ext of gnoblin.extensions) console.log(ext.id, ext.status);
```

XR-specific session and tracking updates may come from extension-host IPC. The JS `gnoblin.extensions.xr` object acts as a typed, observable client-side projection of that service. The host should not expose a half-alive session after a crashed extension; session lifetimes and events must be explicit.

---

## 18. Runtime lifecycle, hot reload, recovery and observability

The JS runtime is a **restartable privileged compositor client**, not code embedded in the compositor's event loop. The compositor and session lock continue operating if it crashes. Normal configuration scripts live under `~/.config/gnoblin/` and may use shipped JS modules without installing Node.js. The runtime enforces CPU, memory and event-handler budgets. The shell cannot accidentally block the GPU render thread while evaluating a poorly written callback.

### Hot reload contract

1. Stage the complete new `init.js` module graph in a fresh JS runtime generation.
2. Recreate layout/binding/listener objects and inherited style declarations in staging; validate imports, permissions, classes, output/display assignments and transactions.
3. Reconcile objects carrying stable user IDs (`Workspace({id})`, named display groups, scene nodes) with the persistent native graph, preserving open clients, window membership, manually adjusted floating positions and relevant workspace state unless explicitly overridden.
4. At one compositor boundary, commit the staged changes atomically; detach listeners and input bindings from the previous generation; run `onCancel()` on any grabs that cannot migrate.
5. On failure, keep the entire previous generation and last valid native scene policy running. Report module path, line, object, field and reason. Do **not** leave only half the new bindings installed.
6. New JS wrappers are generation-scoped; native objects retain stable IDs. Stored JS references from discarded generations must not be called after swap.

**Example:**

```javascript
// Edit this while Gnoblin runs, then use the reload command.
const desktop = new Workspace({ id: "desktop", layout: new FloatingLayout() });
gnoblin.workspaces.add(desktop);
```

The new configuration with the same `id: "desktop"` should **reconcile** with the existing desktop object, rather than destroying its clients or creating a second workspace. Native scene objects and Wayland clients outlive the scripting generation. For advanced scripts, persistent user state should be explicit, typed and keyed rather than relying on module-level JS variables to survive reload.

**Introspection:** a Gnoblin console/inspector is a first-class development tool. It can evaluate JS in the current runtime, inspect the object graph, show inherited styles/computed styles, current transition tracks, binding-match priority, layout placements, output profiles and XR layer plans.

```javascript
// Inspector expressions
gnoblin.clients.length;
gnoblin.clients.focused?.title;
gnoblin.workspaces.map((ws) => [ws.id, ws.clients.length]);
gnoblin.displays.map((d) => [d.id, d.viewport.content.id]);
gnoblin.input.primary.bindings;
gnoblin.extensions.xr?.session?.layerBudget;
gnoblin.clients.focused?.computedStyle.shadows;
```

Input debug mode should show **which binding matched a trigger and why**, and the current per-gesture state (without exposing secrets). The transition inspector should show logical target state, rendered interpolated state, conflicting tracks and completion status.

### Failure and security rules

- Never run arbitrary `.so` in the compositor process by default. Reject ABI mismatches and unauthorised capabilities; crash/restart an extension host independently.
- Respect session lock and input grabs even when user callbacks are late or fail. Native input capture is always released on disconnect.
- Respect Wayland `xdg_toplevel` minimum/maximum sizes, configure/ack/commit ordering and security-sensitive surfaces. A `client.size` assignment requests a configure, not an immediate committed buffer resize.
- Render no private/secure client data into unauthorised `View`, capture, XR or stream outputs; consider all presentations in privacy indicator/audit state.
- Bound effect lists, blur costs, shader execution, imported assets and scene graph depth. Shader support can be an optional validated resource API rather than unrestricted arbitrary GPU code.
- Coalesce input updates from a device when JS is behind, but keep **press/release/cancel** lifecycle events and ordering. A high-rate XR gesture should have instrumentation for missed samples and end-to-end latency.
- XR head tracking and swapchain timing bypass JS and use native fast paths with bounded observer rights. **JS implements manipulation logic**, not headset pose prediction.
- The stock shell must function without any XR extension, without a GPU feature, or without a user `init.js`.

---

## 19. Cookbook: additional concrete use cases

These short independent snippets illustrate the design's breadth. They assume that the relevant workspace, display, client and camera variables exist in the user's scope. They are design examples, not an assertion that every effect is already available in the renderer.

### A. Different styles for focused vs unfocused clients

```javascript
class FocusStyleListener extends ClientListener {
    onFocusChanged(client, focused) {
        gnoblin.requestTransition(
            () => {
                client.style.opacity = focused ? 1 : 0.82;
                client.style.borders = focused ? [{ width: 2, color: "#8b5cf6" }] : [{ width: 1, color: "#ffffff30" }];
            },
            { duration: 180 },
        );
    }
}
gnoblin.clients.addListener(new FocusStyleListener());
```

### B. Different style defaults per workspace

```javascript
gnoblin.clients.style.cornerRadius = 12;
development.clients.style.cornerRadius = 0;
gaming.clients.style.shadows = [{ x: 0, y: 0, blur: 30, color: "#22c55e66" }];
```

### C. Multi-layer border and glow for only terminals

```javascript
const terminals = gnoblin.clients.where({ appId: "com.mitchellh.ghostty" });
terminals.style.borders = [
    { width: 1, color: "#ffffff50" },
    { width: 2, color: "#0ea5e9", offset: 2 },
];
terminals.style.shadows = [{ x: 0, y: 0, blur: 30, spread: 4, color: "#38bdf855" }];
```

### D. Frosted background with multiple filters

```javascript
client.style.backdropFilter = [{ blur: 24 }, { saturate: 1.35 }, { brightness: 1.05 }];
client.style.filter = [{ contrast: 1.1 }];
```

### E. Remove a per-client override and re-inherit global defaults

```javascript
client.style.cornerRadius = 24;
client.style.unset("cornerRadius");
console.log(client.computedStyle.cornerRadius);
```

### F. Class-based application rule without a stylesheet

```javascript
const firefoxClients = gnoblin.clients.where({ appId: "firefox" });
firefoxClients.style.cornerRadius = 18;
firefoxClients.style.backdropFilter = [];
```

### G. Disable resize while keeping default move binding

```javascript
const layout = new FloatingLayout({ triggers: { resize: null } });
workspace.layout = layout;
```

### H. Remap a default layout trigger after creation

```javascript
workspace.layout.bindings.get("move").trigger = "Alt+MouseLeft+Drag";
```

### I. Replace a layout's binding with custom JS behaviour

```javascript
class LockedVerticalMoveBinding extends ClientMoveBinding {
    onUpdate(event, state) {
        const y = state.startBounds.y + event.position.y - state.startPointer.y;
        state.client.bounds = { ...state.startBounds, y };
    }
}
workspace.layout.bindings.remove("move");
workspace.layout.bindings.add(
    new LockedVerticalMoveBinding({
        id: "move",
        trigger: "Super+MouseLeft+Drag",
    }),
);
```

### J. Super + right drag picks initial TL/TR/BL/BR quadrant

```javascript
const layout = new FloatingLayout({
    triggers: { resize: "Super+MouseRight+Drag" },
});
layout.bindings.get("resize").edges = "quadrant";
```

### K. Swap tiled clients with a drag

```javascript
const tiled = new MasterStackLayout({
    triggers: { swap: "Super+MouseLeft+Drag" },
});
workspace.layout = tiled;
// Super+left-drag one Client over another: ClientSwapBinding previews and
// changes the layout's order on release.
```

### L. Bind the same gesture differently in two workspaces

```javascript
desktop.layout = new FloatingLayout();
development.layout = new MasterStackLayout();
// Super+MouseLeft+Drag => move Client in Desktop, swap Client in Development.
```

### M. Switch workspaces only on the display currently under the pointer

```javascript
gnoblin.input.primary.bind("Super+3", () => {
    const display = gnoblin.input.primary.activeDisplay;
    if (display) display.viewport.content = gnoblin.workspaces.get("web");
});
```

### N. Duplicate workspace content on two displays with different cameras

```javascript
leftDisplay.viewport.content = development;
rightDisplay.viewport.content = development;
leftDisplay.viewport.camera = new Camera2D();
rightDisplay.viewport.camera = new Camera3D({ projection: "perspective", fov: 55 });
```

### O. Exact mirror both monitors

```javascript
const mirrored = new Display({
    outputs: [mainProfile, secondProfile],
    mode: "mirror",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
```

### P. Span one workspace across two physical monitors

```javascript
const wide = new Display({
    outputs: [mainProfile, secondProfile],
    mode: "span",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
```

### Q. Put a normal workspace on a perspective monitor

```javascript
display.viewport.camera = new Camera3D({
    projection: "perspective",
    fov: 65,
    position: { x: 0, y: 0, z: 1400 },
});
```

### R. Animate an entire layout from tiles to a 3D carousel

```javascript
gnoblin.requestTransition(
    () => {
        workspace.layout = new ArcLayout({ radius: 1300, spacing: 0.3 });
        display.viewport.camera = new Camera3D({
            projection: "perspective",
            fov: 65,
            position: { x: 0, y: 0, z: 1500 },
        });
    },
    { duration: 750, easing: "ease-in-out" },
);
```

### S. Change a tiling ratio with animation

```javascript
gnoblin.requestTransition(
    () => {
        development.layout.ratio = 0.72;
    },
    { duration: 300 },
);
```

### T. Animate the whole global border style, with an ordered stack

```javascript
gnoblin.requestTransition(
    () => {
        gnoblin.clients.style.borders[0].width = 3;
        gnoblin.clients.style.shadows[0].blur = 36;
    },
    { duration: 250, easing: "ease-out" },
);
```

### U. Use XR without XR APIs in the core camera class

```javascript
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    const head = new xr.Camera3D({ tracking: xr.session.head });
    xr.session.viewport = new xr.Viewport({
        content: room,
        camera: head,
        composition: "auto",
    });
}
```

### V. Disable a default XR grip interaction

```javascript
const xr = gnoblin.extensions.xr;
const spatial = new xr.SpatialLayout({ triggers: { move: null } });
```

### W. XR ray-drag instead of following the controller's full pose

```javascript
const xr = gnoblin.extensions.xr;
spatial.bindings.remove("move");
spatial.bindings.add(
    new xr.ViewMoveBinding({
        id: "move",
        trigger: "Grip+Drag",
        tracking: "ray",
        rotation: "keep",
    }),
);
```

### X. XR preview resize, defer real client configure until release

```javascript
const xr = gnoblin.extensions.xr;
spatial.bindings.add(
    new xr.ViewResizeBinding({
        id: "resize",
        trigger: "Grip+Trigger+Drag",
        pixelsPerMetre: 1200,
        edges: "quadrant",
    }),
);
// During drag: View.size changes.
// On release: Client.size request if aspect ratio/density requires it.
```

### Y. Head-locked Bingux panel without coupling Bingux to Gnoblin XR internals

```javascript
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    const hud = new VirtualOutput({
        name: "XR-HUD",
        width: 1600,
        height: 900,
        attachment: "viewer",
    });
    gnoblin.outputs.add(hud);
    // Bingux chooses XR-HUD as an ordinary layer-shell target.
}
```

### Z. Observe output mode and hotplug state

```javascript
class LogOutputs extends OutputListener {
    onModeChanged(output, mode) {
        console.log(output.connector, mode);
    }
    onDisconnected(output) {
        console.log("Unplugged", output.connector);
    }
}
gnoblin.outputs.addListener(new LogOutputs());
```

### AA. Input context with different shortcuts for a second user/controller

```javascript
const other = new InputContext({ name: "secondary" });
gnoblin.input.contexts.add(other);
other.bind("Super+Return", () => gnoblin.spawn("foot"));
```

### AB. Responsive style from scene state (tracked property binding)

```javascript
client.properties.bind("opacity", () => {
    const d = client.position.distanceTo(camera.position);
    return Math.max(0.35, 1 - d / 3000);
});
```

### AC. Own 3D layout without any `.so`

```javascript
class HelixLayout extends Layout {
    constructor({ radius = 800, step = 130, angle = 0.55 } = {}) {
        super();
        Object.assign(this, { radius, step, angle });
    }
    compute({ children }) {
        return children.map((node, i) => ({
            node,
            transform: {
                position: {
                    x: Math.sin(i * this.angle) * this.radius,
                    y: i * this.step,
                    z: -Math.cos(i * this.angle) * this.radius,
                },
                rotation: { x: 0, y: -i * this.angle, z: 0 },
            },
        }));
    }
}
```

### AD. Scrub a workspace transition with a touchpad

```javascript
const t = gnoblin.requestTransition(
    () => {
        display.viewport.content = web;
    },
    { duration: 400, autoplay: false },
);
// A GestureBinding can set t.progress from 0..1; release calls play/reverse.
```

### AE. One-off atomic native state update, without animation

```javascript
client.set({ opacity: 1, visible: true, transform: initialTransform });
```

### AF. Apply same transition to two clients

```javascript
gnoblin.requestTransition(
    () => {
        firefox.style.opacity = 1;
        terminal.style.opacity = 0.5;
    },
    { duration: 220 },
);
```

### AG. Workspaces with no visible output are still first-class

```javascript
const background = new Workspace({ id: "background", layout: new MonocleLayout() });
gnoblin.workspaces.add(background);
// background.clients still exists; nothing is rendered until a Viewport references it.
```

### AH. Remove a binding entirely without subclassing a layout

```javascript
const grid = new GridLayout();
grid.bindings.remove("reorder");
```

---

## 20. Full reference `init.js` — everything together

This example is deliberately self-contained enough to serve as a target configuration for an implementation. Its imports refer to the standard classes and to user modules defined elsewhere in this specification. Optional XR is guarded; a user without a headset should get the same ordinary desktop.

```javascript
// ~/.config/gnoblin/init.js
import {
    ClientListener,
    Node,
    View,
    Workspace,
    FloatingLayout,
    MasterStackLayout,
    GridLayout,
    ArcLayout,
    OutputProfile,
    Display,
    Viewport,
    Camera2D,
    Camera3D,
} from "gnoblin";

// ── 1. Styles: global defaults, real OOP properties ──────────────────────
gnoblin.clients.style.cornerRadius = 12;
gnoblin.clients.style.borders = [
    { width: 1, color: "#ffffff40" },
    { width: 2, color: "#7c3aed", offset: 3 },
];
gnoblin.clients.style.shadows = [
    { x: 0, y: 8, blur: 24, color: "#00000066" },
    { x: 0, y: 0, blur: 12, spread: 2, color: "#8b5cf655" },
    { inset: true, x: 0, y: 0, blur: 4, color: "#ffffff22" },
];
gnoblin.clients.style.backdropFilter = [{ blur: 18 }, { saturate: 1.25 }];
gnoblin.clients.style.filter = [{ brightness: 1.05 }, { contrast: 1.1 }];

// ── 2. Workspaces own layouts and clients ─────────────────────────────────
const desktop = new Workspace({
    id: "desktop",
    name: "Desktop",
    layout: new FloatingLayout({
        placement: "smart",
        triggers: { move: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" },
    }),
});
const development = new Workspace({
    id: "development",
    name: "Development",
    layout: new MasterStackLayout({
        ratio: 0.65,
        gap: 12,
        triggers: { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" },
    }),
});
const web = new Workspace({
    id: "web",
    name: "Web",
    layout: new GridLayout({ columns: "auto", gap: 8 }),
});
for (const ws of [desktop, development, web]) gnoblin.workspaces.add(ws);

// A workspace can override collection defaults without mutating every client.
development.clients.style.cornerRadius = 4;
web.clients.style.backdropFilter = [];

// Live collection selection is another style inheritance layer.
gnoblin.clients.where({ appId: "firefox" }).style.cornerRadius = 18;

// ── 3. Hardware profiles survive monitor hotplug ─────────────────────────
const main = new OutputProfile({
    match: { connector: "DP-1" },
    mode: { width: 3440, height: 1440, refreshRate: 144 },
    scale: 1.25,
    position: { x: 0, y: 0 },
});
const second = new OutputProfile({
    match: { connector: "HDMI-A-1" },
    mode: "preferred",
    position: { rightOf: main, align: "top" },
});
gnoblin.outputs.profiles.add(main);
gnoblin.outputs.profiles.add(second);

// ── 4. Logical displays decide WHAT hardware presents ───────────────────
const mainDisplay = new Display({
    id: "main-display",
    outputs: [main],
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
const sideDisplay = new Display({
    id: "side-display",
    outputs: [second],
    viewport: new Viewport({ content: development, camera: new Camera2D() }),
});
gnoblin.displays.add(mainDisplay);
gnoblin.displays.add(sideDisplay);

// To show the same workspace independently on both monitors:
// sideDisplay.viewport.content = desktop;
// To mirror exact pixels or span the desktops, create one Display({
//   outputs: [main, second], mode: "mirror" | "span", viewport: ...
// }) instead of two independently owned displays.

// ── 5. InputContext: global keyboard commands, layout-owned drag binds ───
const input = gnoblin.input.primary;
input.bind("Super+Return", () => gnoblin.spawn("ghostty"));
input.bind("Super+Q", () => input.focusedClient?.close());
input.bind("Super+1", () => {
    if (input.activeDisplay) input.activeDisplay.viewport.content = desktop;
});
input.bind("Super+2", () => {
    if (input.activeDisplay) input.activeDisplay.viewport.content = development;
});
input.bind("Super+3", () => {
    if (input.activeDisplay) input.activeDisplay.viewport.content = web;
});

// No need for input.bind("Super+MouseLeft+Drag", ...): each layout registered
// its default ClientMoveBinding / ClientSwapBinding in its constructor.

// ── 6. Typed client listeners and animated styles ────────────────────────
class FocusAppearance extends ClientListener {
    onFocusChanged(client, focused) {
        gnoblin.requestTransition(
            () => {
                client.style.opacity = focused ? 1 : 0.86;
            },
            { duration: 180, easing: "ease-out" },
        );
    }
}
gnoblin.clients.addListener(new FocusAppearance());

// ── 7. 2D ↔ 3D desktop with a native coordinated transition ──────────────
const flatLayout = desktop.layout;
const arcLayout = new ArcLayout({ radius: 1200, spacing: 0.35 });
const camera2D = mainDisplay.viewport.camera;
const camera3D = new Camera3D({
    projection: "perspective",
    fov: 65,
    position: { x: 0, y: 0, z: 1600 },
});
let threeD = false;
input.bind("Super+Tab", () => {
    threeD = !threeD;
    gnoblin.requestTransition(
        () => {
            desktop.layout = threeD ? arcLayout : flatLayout;
            mainDisplay.viewport.camera = threeD ? camera3D : camera2D;
            gnoblin.clients.style.shadows[0].blur = threeD ? 42 : 24;
        },
        { duration: 750, easing: "ease-in-out" },
    );
});

// ── 8. Optional XR extension, no mandatory XR core API ───────────────────
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    const room = new Node({ id: "xr-room" });
    room.metresPerUnit = 1;
    room.layout = new xr.SpatialLayout({
        radius: 1.6,
        spacing: 0.35,
        triggers: { move: "Grip+Drag", resize: "Grip+Trigger+Drag" },
    });

    // Real application to room population belongs in a reusable helper.
    for (const client of gnoblin.clients) {
        if (client.secure) continue;
        const aspect = client.committedSize.width / client.committedSize.height;
        room.add(
            new View({
                content: client,
                size: { width: 0.8, height: 0.8 / aspect },
                input: "forward",
                contentFit: "contain",
            }),
        );
    }
    gnoblin.scene.root.add(room);
    const head = new xr.Camera3D({ tracking: xr.session.head });
    xr.session.viewport = new xr.Viewport({
        content: room,
        camera: head,
        composition: "auto",
    });
}
```

In a production configuration, factor room creation into a separate module that observes `gnoblin.clients` and creates/destroys views for clients that arrive or leave while the XR session is active. The lifecycle example in §13 shows the relevant pattern.

---

## 21. Implementation contracts and acceptance tests

The following tests make this document more than a stylistic sketch. Each is phrased as an observable contract an implementer can verify, whether against a fake scene model, a headless Smithay compositor or a simulated OpenXR runtime.

### JS object model

- `gnoblin` is the same global object in all imported user modules.
- `Client`, `Workspace`, `Display`, `Viewport`, `Camera2D`, `Camera3D`, `Layout` and listener classes are importable from the shipped `gnoblin` module with generated declarations.
- `gnoblin.clients` and `workspace.clients` reference the same native `Client` object; removing a workspace membership does not destroy the client.
- A subclass of `Layout` with ordinary `this.radius` field can be constructed, assigned to `workspace.layout`, changed, observed and animated without user-written proxy/decorator/schema metadata.
- `Collection<T>.where()` remains live across additions/removals and app ID changes. `.filter()` returns a static JS array.
- Collection listener forwarding attaches exactly once to every current/future member and unsubscribes on removal; origin object is always callback argument 1.

### Styles

- `client.style.cornerRadius` affects only the selected client; setting `gnoblin.clients.style.cornerRadius` sets an inherited default for current and future clients without overwriting explicit individual overrides.
- Workspace and `where()` selection styles have deterministic per-property precedence and removal fallback.
- Borders and shadows support zero, one or several ordered typed layers; `client.style.shadows[0].blur = 20` is tracked natively.
- Content filter and backdrop filter are distinct; backdrop blur cannot leak secure pixels.
- `client.computedStyle` exposes actual resolved style and origin; inherited animated layers do not duplicate properties on every client.

### Displays and workspaces

- One `Workspace` can be displayed through two independent `Display` objects with different cameras without duplicating clients or changing their logical geometry twice.
- `Display(mode:"mirror")` presents the same image on two outputs as one logical monitor.
- `Display(mode:"span")` presents different clipped regions of one workspace through the logical union of outputs; pointer coordinates and damage mapping remain correct across boundaries.
- Two independent displays switch workspace selection independently through `display.viewport.content`.
- OutputProfile hotplug and modeset failures roll back atomically, preserving a usable last-good display arrangement.
- One physical output is not silently driven by two active displays at once.

### Binding lifecycle and layouts

- `new FloatingLayout()` installs move and resize bindings with the documented default triggers.
- Constructor `triggers.resize = null` omits only that binding; `.bindings.remove("move")` removes it without disabling the layout itself.
- `Super+MouseRight+Drag` chooses TL/TR/BL/BR from **initial** local pointer position, and does not change edges mid-drag.
- Fullscreen policy declines moving/resizing when necessary; maximised policy is explicit and deterministic.
- `ClientMoveBinding`, `ClientResizeBinding`, `ClientSwapBinding` and `SplitResizeBinding` execute their `onStart/onUpdate/onEnd/onCancel` policy in JS and use only generic Rust scene/input services.
- Simultaneous drags from independent input contexts have separate gesture state; cancelling one cannot clobber the other's state.
- Input capture is released even when the runtime fails mid-grab; user clients do not keep a stuck mouse button.
- `FloatingLayout` preserves user-set floating bounds when a different client opens; tiling layouts recompute atomically when ratios or membership change.

### Animation

- `requestTransition` callback executes **once**; multiple tracked property changes commit as one native transaction.
- A 2D layout to 3D layout transition interpolates geometry by client ID and blends the camera projection with no abrupt clipping/scale jump at the transition boundary.
- Layout, border, shadow, filter, opacity, camera and transform changes can be coordinated by one transition.
- Layout param transitions don't require JavaScript rendering on every compositor frame.
- Hit testing uses presented intermediate transforms rather than target geometry while an animation is running.
- Gesture-scrubbable `Transition.progress` works in both directions and settles cleanly on release/cancel.
- Invalid or stale target values reject the whole transition while preserving the previous valid state.

### XR

- XR is absent/optional without affecting a standard desktop; `gnoblin.extensions.xr` correctly represents loaded service state.
- The XR `.so` registers its classes and capabilities only through a versioned ABI; a failing extension host doesn't terminate the compositor.
- `XRCamera3D` is a subtype of `Camera3D`, and `XRViewport` is a subtype of `Viewport`; ordinary core objects have no OpenXR-specific configuration.
- Controller ray hit-testing identifies a `View` and forwards normal selection input to its source client when no compositor binding consumes the trigger.
- `ViewMoveBinding` changes view transform while leaving the source client's desktop position intact.
- `ViewResizeBinding` changes **physical panel size** on input updates and sends **no Wayland configure requests during the drag**; it submits at most one new target client size on successful release when aspect/density thresholds demand it.
- When a client has multiple views, the logical size request affects the one source client; all views continue rendering safely while a new buffer is pending.
- XR tracking, compositor frame scheduling, and OpenXR swapchain/presentation don't wait on JavaScript callbacks; a failed JS binding cancels safely.
- Real XR headset validation is kept separate from mock/null OpenXR tests; no headset capability is claimed proven solely by a simulator.

---

## 22. Suggested migration sequence

This is a **dependency order**, not a requirement to rewrite all of Gnoblin in one effort.

| Stage | Primary work                                                                           | Demonstrable outcome                                                                     |
| ----- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| 1     | New ES module runtime and generated native bindings, `gnoblin` global, `Client` rename | Simple `init.js` can inspect native clients and register a typed listener                |
| 2     | `Collection<T>` and OOP `Workspace`, layout binding and identity reconciliation        | Multiple real workspaces with persistent clients and layouts                             |
| 3     | Native style objects and collection inheritance, multiple borders/shadows/filters      | Global style, scoped style and one-client overrides work and animate                     |
| 4     | `OutputProfile`, `Display` and `Viewport` refactor                                     | Independent/same-workspace/mirror/span modes with correct pointer and rendering          |
| 5     | `Layout.compute` and lifecycle hooks; JS binding framework                             | Floating, tiling, grid and custom layouts work entirely from shipped JS policy           |
| 6     | Native-backed property transition graph and `requestTransition`                        | Composite camera/layout/style transition, scrubbing and failure recovery                 |
| 7     | Extension host, `.so` ABI, capability grants                                           | Load/unload optional XR implementation without compositor dependency                     |
| 8     | XR camera/viewport and JS manipulation bindings                                        | Six-axis move, spatial resize with client configure on release, simulated headset proofs |
| 9     | Hardware XR validation, performance, security, hot-reload stress                       | Measured latency, robust recovery, real compositor/headset compatibility                 |

Keep the existing Rust scene, output, tween and transaction code wherever possible; change the binding and policy surface incrementally. A parity/test harness can run the old Lua layout corpus and the new JS stock layouts against identical geometry cases to catch regressions. The Lua shell can remain as a migration reference until the JS shell reaches parity, but **users should end up authoring one JS environment for both configuration and behaviour**.

---

## 23. Summary API glossary

| Identifier                                 | Contract                                                                         |
| ------------------------------------------ | -------------------------------------------------------------------------------- |
| `gnoblin`                                  | Built-in global for one session's JS environment                                 |
| `Client`                                   | Managed Wayland toplevel, with geometry, style and focus events                  |
| `gnoblin.clients`                          | `Collection<Client>` of all managed toplevels                                    |
| `Workspace`                                | OOP node containing managed clients and layout policy                            |
| `gnoblin.workspaces`                       | Global `Collection<Workspace>`                                                   |
| `workspace.clients`                        | Live reference `Collection<Client>` for that workspace                           |
| `gnoblin.clients.style`                    | Inherited global client-style object                                             |
| `workspace.clients.style`                  | Workspace-specific inherited defaults                                            |
| `client.style`                             | Explicit style overrides for only that client                                    |
| `client.computedStyle`                     | Read-only effective/computed appearance                                          |
| `Style`, `ClientStyle`, `Border`, `Shadow` | Native-backed style object hierarchy                                             |
| `Scene`, `Node`                            | Native spatial scene graph and owned nodes                                       |
| `View`                                     | Scene node that renders content referenced elsewhere                             |
| `Camera2D`, `Camera3D`                     | Orthographic/perspective projection classes                                      |
| `Viewport`                                 | Camera + content to be rendered                                                  |
| `Output`                                   | Connected physical or virtual monitor connector                                  |
| `OutputProfile`                            | Persistent desired hardware configuration                                        |
| `Display`                                  | One/many outputs presenting a viewport (`single`, `mirror`, `span`)              |
| `gnoblin.outputs`, `gnoblin.displays`      | Collections of hardware and logical presentations                                |
| `InputContext`                             | Logical group of physical devices and input/focus state; Wayland seat equivalent |
| `gnoblin.input.primary`                    | Primary input context                                                            |
| `Binding`                                  | Named input trigger + behaviour; layout-scoped or context-scoped                 |
| `PointerDragBinding`, `GestureBinding`     | Reusable JS lifecycle bases with native event delivery                           |
| `ClientMoveBinding`, `ClientResizeBinding` | JS stock floating interactions                                                   |
| `ClientSwapBinding`, `SplitResizeBinding`  | JS stock tiling interactions                                                     |
| `Layout.compute(context)`                  | Pure desired geometry calculation on invalidation                                |
| `Layout.onClientAdded(...)`                | Optional one-off arrival logic for floating placement                            |
| `gnoblin.requestTransition(...)`           | Atomic, animatable native state transition                                       |
| `Transition`                               | Time-driven or gesture-controlled native animation object                        |
| `gnoblin.extensions.xr`                    | Optional XR service loaded through `.so` extension host                          |
| `XRCamera3D`, `XRViewport`                 | XR-only extensions of ordinary camera/viewport                                   |
| `ViewMoveBinding`, `ViewResizeBinding`     | JavaScript XR interactions manipulating views                                    |

### One final non-goal

Gnoblin **does not** become Bingux's UI toolkit. Bingux is a separate Wayland layer-shell client; it can consume regular outputs or an XR viewer-attached output if it chooses. Gnoblin owns compositor-side scene geometry, input, policy and presentation. The two programs may communicate through optional IPC and Wayland protocols, but their implementation languages and rendering APIs remain independent.

**End of specification.**
