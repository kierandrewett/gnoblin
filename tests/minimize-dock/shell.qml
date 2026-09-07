import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

PanelWindow {
    id: dock
    implicitWidth: 400
    implicitHeight: 100
    anchors.bottom: true
    exclusiveZone: 0
    color: "#222222"
    WlrLayershell.layer: WlrLayer.Top
    WlrLayershell.namespace: "gnoblin-target-test"

    function application() {
        return ToplevelManager.toplevels.values.find(t => t.title === "Gnoblin target test");
    }

    IpcHandler {
        target: "dock"
        function setTarget(x: int, y: int): string {
            const app = dock.application();
            if (!app)
                return "waiting";
            app.setRectangle(dock, Qt.rect(x, y, 32, 40));
            return "ready";
        }
        function moveDock(): void { dock.margins.bottom = 50; }
        function clearTarget(): void { dock.application().setRectangle(dock, Qt.rect(0, 0, 0, 0)); }
        function minimize(): void { dock.application().minimized = true; }
        function restore(): void {
            dock.application().minimized = false;
            dock.application().activate();
        }
    }
}
