import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

test("opaque buffers use compositor metadata without GPU readback", () => {
    const source = readFileSync(
        new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinCorners.js", import.meta.url),
        "utf8",
    );
    const context = vm.createContext({ Geometry: { geometry: (frame) => frame } });
    vm.runInContext(
        source.slice(source.indexOf("const visibleFrames ="), source.indexOf("const declarations =")),
        context,
    );
    let reads = 0;
    const rectangle = { x: 10, y: 20, width: 800, height: 600 };
    const actor = {
        width: 800,
        height: 600,
        mapped: true,
        opacity: 255,
        get_resource_scale: () => 1,
        get_texture: () => ({ get_texture: () => ({}), is_opaque: () => true }),
        get_image: () => {
            reads++;
            throw new Error("Unexpected GPU readback");
        },
        meta_window: { get_frame_rect: () => rectangle, get_buffer_rect: () => rectangle },
    };
    const config = { padding: [0, 0, 0, 0] };
    assert.equal(context.windowGeometry(actor, actor, config).width, 800);
    rectangle.width = 1200;
    actor.width = 1200;
    assert.equal(context.windowGeometry(actor, actor, config).width, 1200);
    assert.equal(reads, 0);
});
