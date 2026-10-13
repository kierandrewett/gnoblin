import QtQuick
import Quickshell

ShellRoot {
    Timer {
        interval: 50
        repeat: true
        running: true
        onTriggered: {
            const entry = DesktopEntries.byId("org.gnoblin.ActionIdentity");
            if (!entry)
                return;
            const actions = [];
            for (const action of entry.actions)
                actions.push({
                    id: action.id,
                    name: action.name
                });
            console.info("GNOBLIN_ACTION_IDENTITY=" + JSON.stringify({
                id: entry.id,
                actions: actions
            }));
            Qt.quit();
        }
    }
}
