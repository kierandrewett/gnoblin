# The shell reloads `~/.config/gnoblin/shell.py` when you save it.
# It runs top to bottom against live objects such as `Workspaces`, `Gnoblin.get().input` and `Gnoblin.get().clients`.
# Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
# This file does what a shell with no shell file does, so a copy you leave unchanged behaves like the default.
# Check changes with `gnoblinctl check shell.py`; see the shell API pages for each object's details.

from gnoblin import gnoblin
from gnoblin import Gnoblin
from gnoblin.std import (
    Decorations,
    FloatingLayout,
    InputContextAugment,
    Launcher,
    MoveBinding,
    NotRespondingAlerts,
    ResizeBinding,
    ShowOptions,
    StripLayout,
    WindowMenus,
    WindowState,
    Workspace,
    Workspaces,
    WorkspaceSlideBinding,
)

# To use the examples below, add what they name to the import above.
# from gnoblin.std import MasterStackLayout

# Four floating workspaces. The first is shown until another is selected.
# Choose a layout: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout or StripLayout.
for workspace_id in ["1", "2", "3", "4"]:
    Workspaces.get().add(Workspace(workspace_id, name=f"Workspace {workspace_id}", layout=FloatingLayout()))

# Workspaces.get().add(Workspace("work", name="Work", layout=MasterStackLayout()))

# Add a titlebar to each window, the window menu and the not-responding alert, and answer a window that asks to be
# maximised, minimised or made fullscreen. Remove a call to disable that part.
# The library moves the keyboard focus on its own, as the seat's focus settings say: a click or a new window takes it.
Decorations.get().start()
WindowMenus.get().start()
NotRespondingAlerts.get().start()
WindowState.get().start()

input = Gnoblin.get().input.primary


# Super+1 .. Super+9 show the workspace at that position on the display under the pointer. A key past the last
# workspace does nothing.
def show_workspace(position):
    def action(_event):
        items = Workspaces.get().list
        if position <= len(items):
            items[position - 1].show(ShowOptions(duration=250.0))

    return action


for position in range(1, 10):
    input.bind(f"Super+{position}", show_workspace(position))


# Super+Left and Super+Right move focus between StripLayout columns on the display under the pointer. They do nothing
# on other layouts.
def move_column(side):
    def action(_event):
        display = InputContextAugment.get().active_display(input)
        viewport = display.viewport if display is not None else None
        layout = viewport.content.layout if viewport is not None and viewport.content is not None else None
        if isinstance(layout, StripLayout):
            getattr(layout, side)()

    return action


input.bind("Super+Left", move_column("focus_left"))
input.bind("Super+Right", move_column("focus_right"))

# Three-finger sideways swipes switch workspaces.
WorkspaceSlideBinding(trigger="Swipe3").install()

# Super with the left button drags a window, and Super with the right button resizes it from the nearest corner.
MoveBinding().install()
ResizeBinding().install()

# Volume, microphone and media player keys: each key runs one command.
media_keys = {
    "XF86AudioRaiseVolume": ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
    "XF86AudioLowerVolume": ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
    "XF86AudioMute": ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
    "XF86AudioMicMute": ["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
    "XF86AudioPlay": ["playerctl", "play-pause"],
    "XF86AudioNext": ["playerctl", "next"],
    "XF86AudioPrev": ["playerctl", "previous"],
}


def run_command(command):
    def action(_event):
        gnoblin.spawn(command)

    return action


for key, command in media_keys.items():
    input.bind(key, run_command(command))


# Super+space opens the launcher, with a command entry in the middle of the screen.
def open_launcher(_event):
    Launcher.open()


input.bind("Super+space", open_launcher)

# Add custom key bindings here. A key already bound above is refused; remove that binding first.
# input.bind("Super+Return", run_command(["foot"]))
# input.bind("Super+q", lambda _event: input.focused_client().close())

# Set a style property to change how each window looks.
# Gnoblin.get().clients.style.corner_radius = 12
