// The documentation sidebar, converted once from the VitePress sidebar in infra/website/.vitepress/config.mts.
// Section and page order are the same; labels are in sentence case. `slug` is the page's path under docs/ without
// `.md` ("" is docs/index.md, "recipes" is docs/recipes/index.md). An item with `items` is a group; it links to
// its own page only when it has a `slug`.
//
// Every page in docs/ must appear here or in UNLISTED, otherwise the build fails (see src/lib/server/docs.ts).

export interface NavItem {
    label: string;
    slug?: string;
    items?: NavItem[];
}

export interface NavSection {
    id: "start" | "shell" | "config" | "dev" | "contrib";
    label: string;
    items: NavItem[];
}

export const NAV: NavSection[] = [
    {
        id: "start",
        label: "Get started",
        items: [
            { label: "Welcome", slug: "" },
            {
                label: "Install",
                items: [
                    { label: "Choose your system", slug: "installation" },
                    { label: "Package repositories", slug: "package-repositories" },
                    { label: "Fedora", slug: "install-fedora" },
                    { label: "Arch Linux", slug: "install-arch" },
                    { label: "Debian", slug: "install-debian" },
                    { label: "NixOS", slug: "install-nixos" },
                    { label: "openSUSE", slug: "install-opensuse" },
                    { label: "Alpine", slug: "install-alpine" },
                    { label: "Build from source", slug: "install-source" },
                    { label: "Optional packages", slug: "optional-packages" },
                ],
            },
            { label: "Platform support", slug: "platform-support" },
            { label: "Troubleshooting", slug: "troubleshooting" },
        ],
    },
    {
        id: "shell",
        label: "Shell and userspace",
        items: [
            { label: "Choose a shell", slug: "bring-your-own-shell" },
            { label: "Bingux", slug: "bingux" },
            { label: "Build a desktop", slug: "build-a-desktop" },
            { label: "GNOME applications", slug: "gnome-apps" },
        ],
    },
    {
        id: "config",
        label: "Configure",
        items: [
            { label: "Config API", slug: "config" },
            {
                label: "gnoblin.configure",
                slug: "config/configure",
                items: [
                    { label: "Window management", slug: "config/configure/window_management" },
                    {
                        label: "Input",
                        slug: "config/configure/input",
                        items: [
                            { label: "Keyboard", slug: "config/configure/input/keyboard" },
                            { label: "Mouse", slug: "config/configure/input/mouse" },
                            { label: "Touchpad", slug: "config/configure/input/touchpad" },
                            {
                                label: "Trackball and pointing stick",
                                slug: "config/configure/input/trackball-pointing-stick",
                            },
                        ],
                    },
                    { label: "Keyboard bindings", slug: "config/configure/keybindings" },
                    { label: "Shortcuts", slug: "config/configure/shortcuts" },
                    { label: "Input sources", slug: "config/configure/input_sources" },
                    { label: "Touchpad gestures", slug: "config/configure/touchpad_gestures" },
                    { label: "Monitors", slug: "config/configure/monitors" },
                    { label: "Portals", slug: "config/configure/portals" },
                    { label: "Location", slug: "config/configure/location" },
                    { label: "Authentication", slug: "config/configure/auth" },
                    { label: "Prompts", slug: "config/configure/prompts" },
                    { label: "Autostart", slug: "config/configure/autostart" },
                    { label: "Compositor", slug: "config/configure/compositor" },
                    { label: "Cursor", slug: "config/configure/cursor" },
                    { label: "Frame renderers", slug: "config/configure/frame_renderers" },
                    { label: "Layer shell", slug: "config/configure/layer_shell" },
                    { label: "Protocols", slug: "config/configure/protocols" },
                    { label: "Shell", slug: "config/configure/shell" },
                    { label: "XWayland", slug: "config/configure/xwayland" },
                ],
            },
            {
                label: "Lua config API",
                items: [
                    { label: "Window rules", slug: "config/window_rule" },
                    { label: "Animation", slug: "config/animation" },
                    { label: "Config", slug: "config/config" },
                    { label: "Load", slug: "config/load" },
                    { label: "Snapshot", slug: "config/snapshot" },
                    { label: "Array", slug: "config/array" },
                ],
            },
            {
                label: "Guides",
                items: [
                    { label: "Files and load order", slug: "guides/files_and_load_order" },
                    { label: "Editor support", slug: "guides/editor_support" },
                    { label: "Shortcuts", slug: "guides/shortcuts" },
                    { label: "Autostart", slug: "guides/autostart" },
                    { label: "Appearance", slug: "guides/theming" },
                    { label: "Wallpapers", slug: "guides/wallpapers" },
                    { label: "Window rules", slug: "guides/window_rules" },
                    { label: "Window effects", slug: "guides/window_effects" },
                    { label: "Window frames", slug: "guides/window_frames" },
                    { label: "Animations", slug: "guides/animations" },
                    { label: "Shaders", slug: "guides/shaders" },
                    { label: "Cursors", slug: "guides/cursors" },
                    { label: "Session settings", slug: "guides/session_settings" },
                    { label: "Window menu", slug: "guides/window_menu" },
                    { label: "Window state shortcuts", slug: "guides/window_state_shortcuts" },
                    { label: "Window snapping", slug: "guides/window_snapping" },
                ],
            },
            {
                label: "Recipes",
                slug: "recipes",
                items: [
                    { label: "Complete starter config", slug: "recipes/complete-starter-config" },
                    { label: "Add a shortcut", slug: "recipes/add-a-shortcut" },
                    { label: "Dim unfocused windows", slug: "recipes/dim-unfocused-windows" },
                    { label: "Round application windows", slug: "recipes/round-application-windows" },
                    { label: "Turn off layer animations", slug: "recipes/turn-off-layer-animations" },
                    { label: "Request a titlebar", slug: "recipes/request-a-titlebar" },
                    { label: "Combine window rules", slug: "recipes/combine-window-rules" },
                    { label: "Split config files", slug: "recipes/split-config-into-files" },
                    { label: "Writing workspace", slug: "recipes/writing-workspace" },
                    { label: "Small desktop", slug: "recipes/small-desktop" },
                    { label: "Laptop input", slug: "recipes/laptop-input" },
                    { label: "Pointer feedback", slug: "recipes/pointer-feedback" },
                    { label: "Pen display", slug: "recipes/pen-display-workflow" },
                    { label: "Organise config files", slug: "recipes/organize-config-files" },
                    { label: "Named workspaces", slug: "recipes/put-apps-on-named-workspaces" },
                    { label: "Springy entrance", slug: "recipes/springy-app-entrance" },
                ],
            },
        ],
    },
    {
        id: "dev",
        label: "Shell development",
        items: [
            { label: "Integration guide", slug: "shell-integration" },
            { label: "Lua event handlers", slug: "user-scripts" },
            {
                label: "Lua runtime API",
                slug: "shell-api",
                items: [
                    { label: "Windows", slug: "shell-api/windows" },
                    { label: "Focus and activation", slug: "shell-api/focus-activation" },
                    { label: "Workspaces and monitors", slug: "shell-api/workspaces-monitors" },
                    { label: "Outputs", slug: "shell-api/outputs" },
                    { label: "Layer surfaces", slug: "shell-api/layer-surfaces" },
                    { label: "Input", slug: "shell-api/input" },
                    { label: "Input handlers", slug: "shell-api/input-handlers" },
                    { label: "Text insertion", slug: "shell-api/text-insertion" },
                    { label: "Window snapping", slug: "shell-api/window-snapping" },
                    { label: "Tablet pad help", slug: "shell-api/tablet-pad-help" },
                    { label: "Keyboard shortcuts", slug: "shell-api/keyboard-shortcuts" },
                    { label: "Launch feedback", slug: "shell-api/launch-feedback" },
                    { label: "Session and runtime health", slug: "shell-api/session-runtime" },
                    { label: "Privacy", slug: "shell-api/privacy" },
                    { label: "Authentication prompts", slug: "shell-api/authentication" },
                    { label: "Keyring and GPG prompts", slug: "shell-api/passphrase-prompts" },
                    { label: "Appearance and animations", slug: "shell-api/appearance-animations" },
                    { label: "Shared types", slug: "shell-api/types" },
                    { label: "Errors and results", slug: "shell-api/errors" },
                ],
            },
            {
                label: "Events",
                slug: "events",
                items: [
                    { label: "Mutter", slug: "events/mutter" },
                    { label: "Lifecycle", slug: "events/lifecycle" },
                    { label: "Input and shortcuts", slug: "events/input-shortcuts" },
                    { label: "Session events", slug: "events/session" },
                    { label: "Launch events", slug: "events/launch-portal" },
                    { label: "Desktop state", slug: "events/desktop-state" },
                    { label: "Listen to all", slug: "events/listen-all" },
                ],
            },
            {
                label: "Compositor bridge",
                slug: "bridge",
                items: [
                    { label: "Connection and framing", slug: "bridge/connection" },
                    { label: "Input and focus", slug: "bridge/input-and-focus" },
                    { label: "Sessions and windows", slug: "bridge/session-and-windows" },
                    { label: "Shortcuts and events", slug: "bridge/shortcuts-and-events" },
                    { label: "Privacy", slug: "bridge/privacy" },
                    { label: "Runtime and appearance", slug: "bridge/runtime-and-appearance" },
                    { label: "Examples", slug: "bridge/examples" },
                ],
            },
            {
                label: "gnoblinctl",
                slug: "cli",
                items: [
                    { label: "Lua console", slug: "cli/lua" },
                    { label: "Graphical console", slug: "cli/graphical-console" },
                    { label: "Shell completion", slug: "cli/completion" },
                    { label: "Windows", slug: "cli/windows" },
                    { label: "Input and animations", slug: "cli/input-and-animations" },
                    { label: "Workspaces, monitors, shortcuts", slug: "cli/workspaces-monitors-shortcuts" },
                    { label: "Session and policy", slug: "cli/session-and-policy" },
                ],
            },
            {
                label: "D-Bus interfaces",
                slug: "dbus",
                items: [
                    { label: "Screen saver", slug: "dbus/screensaver" },
                    { label: "Session activity", slug: "dbus/session-idle" },
                ],
            },
            { label: "Wayland protocols", slug: "wayland-protocols" },
            { label: "Background effects", slug: "background-effects" },
            { label: "Frame renderer architecture", slug: "window-frame-renderers" },
            { label: "Frame renderer API", slug: "frame-renderer-api" },
            { label: "Session locking", slug: "session-lock" },
        ],
    },
    {
        id: "contrib",
        label: "Contributing",
        items: [
            { label: "Source development", slug: "source-development" },
            { label: "Devkit", slug: "devkit" },
            { label: "Testing", slug: "testing" },
            { label: "Desktop verification", slug: "real-hardware-verification" },
            { label: "Formatting", slug: "code-quality" },
            { label: "Documentation", slug: "documentation-site" },
        ],
    },
];

/** Pages that exist in docs/ on purpose without a sidebar entry. They are built and linked from other pages. */
export const UNLISTED: string[] = [
    // Short pages that point to where a reference moved; old links still reach them.
    "blur-fades",
    "bridge-examples",
    "compositor-bridge",
    "config/lua-events",
    "config/runtime-api",
    "focus-transfer",
    "launch-feedback",
    "shell-dbus-api",
    // Reference pages reached from other pages rather than the sidebar.
    "config/configure/config-files",
    "gnoblinctl",
];

/** The URL of a docs page. */
export function docsHref(slug: string): string {
    return slug === "" ? "/docs" : `/docs/${slug}`;
}

export interface NavEntry {
    label: string;
    slug: string;
    section: NavSection;
    /** The section and groups above this page, for breadcrumbs. Groups with a page have a slug. */
    trail: { label: string; slug?: string }[];
}

/** Every page in sidebar order, the order the previous and next links follow. */
export const NAV_ORDER: NavEntry[] = NAV.flatMap((section) => {
    const out: NavEntry[] = [];
    const walk = (items: NavItem[], trail: NavEntry["trail"]) => {
        for (const item of items) {
            if (item.slug !== undefined) out.push({ label: item.label, slug: item.slug, section, trail });
            if (item.items) walk(item.items, [...trail, { label: item.label, slug: item.slug }]);
        }
    };
    walk(section.items, [{ label: section.label }]);
    return out;
});
