import {Extension} from "resource:///org/gnome/shell/extensions/extension.js";
import * as Main from "resource:///org/gnome/shell/ui/main.js";

export default class extends Extension {
    enable() {
        const isGnoblin = Main.sessionMode.currentMode === "gnoblin";
        const panelData = Main.layoutManager._trackedActors.find(
            ({actor}) => actor === Main.layoutManager.panelBox,
        );
        const expectedOpacity = isGnoblin ? 0 : 255;
        const expectedReactive = !isGnoblin;

        if (Main.panel.opacity !== expectedOpacity || Main.panel.reactive !== expectedReactive) {
            throw new Error(
                `panel policy mismatch: mode=${Main.sessionMode.currentMode} ` +
                `opacity=${Main.panel.opacity} reactive=${Main.panel.reactive}`,
            );
        }
        if (panelData?.affectsStruts !== expectedReactive ||
            panelData?.affectsInputRegion !== expectedReactive) {
            throw new Error(
                `panel chrome mismatch: mode=${Main.sessionMode.currentMode} ` +
                `struts=${panelData?.affectsStruts} input=${panelData?.affectsInputRegion}`,
            );
        }
    }

    disable() {}
}
