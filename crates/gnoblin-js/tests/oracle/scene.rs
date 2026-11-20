//! The scene (`gnoblin-std/scene/*`, `display/camera2d.ts`, `gnoblin-core/math/*`): vector, pose, ray and plane arithmetic, parsing of
//! positions, rotations and scales, script nodes and their writes, the scene root, cameras and views.
//!
//! Numbers a scenario computes itself (the math table) are rounded to ten decimals so that a native implementation compares them within a
//! tolerance. What the shell writes to the tree is recorded as it is.

use crate::scn::{At, Home, Scn, V, World};

/// A screen of 1000 by 800 and `count` windows of 800 by 600 (ids 3 on, titled `win1`, app id `foot`).
fn world(count: u64) -> World {
    let mut world = World::desktop(1000.0, 800.0).windows(count);
    for n in 0..count {
        world = world
            .with(3 + n, "width", V::Num(800.0))
            .with(3 + n, "height", V::Num(600.0));
    }
    world
}

/// What the scenarios share: `fail` returns the message of what a function throws.
const HELPERS: &str = "globalThis.fail = (f: () => void) => { try { f(); return 'accepted'; } catch (e) { return (e as Error).message; } };\n";

const MATH: &str = "import { Vector3, Ray, Plane } from 'gnoblin-core/math/vector';\n\
    import { Transform3D } from 'gnoblin-core/math/transform';\n\
    import { vectorFrom, rotationFrom, scaleFrom, transformFrom, eulerOfQuaternion } from 'gnoblin-core/math/spatial';\n\
    const r = (n: number) => Math.round(n * 1e10) / 1e10;\n\
    const v = (x: Vector3) => x.toArray().map(r);\n\
    const t = (p: Transform3D) => ({ position: v(p.position), rotation: v(p.rotation), scale: r(p.scale) });\n\
    const attempt = (f: () => unknown) => { try { return f(); } catch (e) { return 'throws: ' + (e as Error).message; } };\n\
    const rounded = (x: unknown): unknown => Array.isArray(x) ? x.map(rounded) : typeof x === 'number' ? r(x) : x;\n\
    const G = globalThis as any;\n\
    G.vectorTable = () => {\n\
        const a = new Vector3(1, 2, 3); const b = new Vector3(-4, 5, 0.5); const z = new Vector3();\n\
        return {\n\
            add: v(a.add(b)), subtract: v(a.subtract(b)), scale: v(a.scale(2.5)), dot: r(a.dot(b)), cross: v(a.cross(b)),\n\
            length: r(a.length()), normalize: v(a.normalize()), zero: attempt(() => v(z.normalize())), distance: r(a.distanceTo(b)),\n\
            from: v(Vector3.from([7, 8, 9])), zeroes: v(Vector3.zero()), clone: v(a.clone()), array: a.toArray(),\n\
            bad_from: attempt(() => v(Vector3.from([1, 2]))), bad_new: attempt(() => v(new Vector3(NaN, 0, 0))),\n\
            parallel: v(new Vector3(0, 0, 2).cross(new Vector3(0, 0, 5))),\n\
        };\n\
    };\n\
    G.rayTable = () => {\n\
        const plane = new Plane(new Vector3(0, 0, -2), new Vector3(0, 0, 1));\n\
        const hit = (o: number[], d: number[]) => {\n\
            const p = plane.intersect(new Ray(new Vector3(o[0], o[1], o[2]), new Vector3(d[0], d[1], d[2])));\n\
            return p === null ? null : v(p);\n\
        };\n\
        const tilted = new Plane(new Vector3(1, 1, 1), new Vector3(1, 1, 1)).intersect(new Ray(new Vector3(0, 0, 0), new Vector3(1, 1, 1)));\n\
        return {\n\
            at: v(new Ray(new Vector3(1, 1, 1), new Vector3(0, 0, -1)).at(3)),\n\
            straight: hit([0, 0, 0], [0, 0, -1]), slanted: hit([1, 2, 0], [0.5, -0.5, -1]), parallel: hit([0, 0, 0], [1, 0, 0]),\n\
            behind: hit([0, 0, -5], [0, 0, -1]), away: hit([0, 0, 0], [0, 0, 1]), on_plane: hit([3, 4, -2], [1, 0, 0]),\n\
            unnormalised: hit([0, 0, 0], [0, 0, -4]), tilted: tilted === null ? null : v(tilted),\n\
        };\n\
    };\n\
    G.poseTable = () => {\n\
        const rotations = [[0, 0, 0], [0.1, 0.2, 0.3], [Math.PI / 2, 0, 0], [0, Math.PI / 2, 0], [0, -Math.PI / 2, 0.3], [3, 2, 1], [0, 0, Math.PI], [-0.7, 1.2, -2.5]];\n\
        const out: unknown[] = [];\n\
        for (const rotation of rotations) {\n\
            for (const scale of [1, 2, 0.5]) {\n\
                const p = new Transform3D(new Vector3(1, -2, 3), rotation, scale);\n\
                const plane = p.plane();\n\
                out.push({\n\
                    rotation: rotation.map(r), scale, euler: v(p.rotation), x: v(p.transformDirection(new Vector3(1, 0, 0))),\n\
                    y: v(p.transformDirection(new Vector3(0, 1, 0))), z: v(p.transformDirection(new Vector3(0, 0, 1))),\n\
                    point: v(p.transformPoint(new Vector3(0.5, 0.25, -1))), plane: [v(plane.point), v(plane.normal)],\n\
                });\n\
            }\n\
        }\n\
        return out;\n\
    };\n\
    G.composeTable = () => {\n\
        const p = new Transform3D(new Vector3(1, 2, 3), [0.3, -0.4, 0.5], 2);\n\
        const q = new Transform3D(new Vector3(-1, 0.5, 2), [1, 0.2, -0.6], 0.5);\n\
        const probe = new Vector3(0.3, -0.2, 0.9);\n\
        const moved = p.clone(); moved.rotation = [0.1, 0.2, 0.3]; moved.position = new Vector3(9, 9, 9); moved.scale = 3;\n\
        return {\n\
            product: t(p.multiply(q)), reverse: t(q.multiply(p)), inverse: t(p.inverse()), round_trip: t(p.multiply(p.inverse())),\n\
            inverse_point: v(p.inverse().transformPoint(p.transformPoint(probe))), identity: t(Transform3D.identity()),\n\
            translation: t(Transform3D.translation(4, 5, 6)), clone: t(p.clone()), mutated_clone: t(moved), original: t(p),\n\
        };\n\
    };\n\
    G.parseTable = () => {\n\
        const triples = [[1, 2, 3], { x: 1, y: 2, z: 3 }, [1, 2], [1, 2, 3, 4], [1, 'a', 3], [1, NaN, 3], [1, Infinity, 3], null, undefined, 'xyz', { x: 1, y: 2 }];\n\
        const rotations = [[0.1, 0.2, 0.3], { x: 0.1, y: 0.2, z: 0.3 }, { x: 0, y: Math.sin(0.25), z: 0, w: Math.cos(0.25) }, { x: 0, y: 0, z: 0, w: 1 },\n\
            { x: 1, y: 0, z: 0, w: 0 }, { x: 0.5, y: 0.5, z: 0.5, w: 0.5 }, { x: 0, y: 0, z: 0, w: 0 }, { x: 0, y: NaN, z: 0 }, 'turn', [1, 2]];\n\
        const scales = [2, [1, 2, 3], { x: 1, y: 2, z: 3 }, 0, -1, 'big', [1, 2], NaN];\n\
        const quaternions = [[0, 0, 0, 1], [Math.sin(Math.PI / 4), 0, 0, Math.cos(Math.PI / 4)], [0, Math.sin(Math.PI / 4), 0, Math.cos(Math.PI / 4)],\n\
            [0, 0, Math.sin(Math.PI / 4), Math.cos(Math.PI / 4)], [0.1826, 0.3651, 0.5477, 0.7303], [0, 0.7071, 0, 0.7071], [0, 1, 0, 0]];\n\
        return {\n\
            vectors: triples.map((value) => attempt(() => rounded(vectorFrom('target', value)))),\n\
            rotations: rotations.map((value) => attempt(() => rounded(rotationFrom('target', value)))),\n\
            scales: scales.map((value) => attempt(() => rounded(scaleFrom('target', value)))),\n\
            quaternions: quaternions.map(([x, y, z, w]) => rounded(eulerOfQuaternion({ x, y, z, w }))),\n\
        };\n\
    };\n\
    G.transformTable = () => {\n\
        const inits = [{ position: { x: 5, y: 6, z: 7 } }, { rotation: [0, 0.5, 0] }, { scale: 2 }, { position: [1, 2, 3], rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: [1, 2, 3] },\n\
            {}, { pose: 1 }, { position: [0, 0, 0], pose: 1 }, { position: { x: 9, y: 9, z: 9 }, rotation: { x: 'no', y: 0, z: 0 } }, 5, null];\n\
        return inits.map((init) => attempt(() => rounded(transformFrom('target', init))));\n\
    };\n";

#[test]
fn math_table() {
    let mut s = Scn::new("scene_math", Home::Reference, &World::desktop(1000.0, 800.0), MATH);
    s.boot();
    s.eval("rec('vector', vectorTable());").step("vectors");
    s.eval("rec('rays', rayTable());").step("rays_and_planes");
    s.eval("rec('poses', poseTable());").step("poses");
    s.eval("rec('compose', composeTable());").step("composition");
    s.eval("rec('parsing', parseTable());").step("parsing");
    s.eval("rec('transforms_from', transformTable());").step("transforms");
    s.finish();
}

#[test]
fn nodes_and_writes() {
    let mut s = Scn::new("scene_nodes", Home::Reference, &world(1), HELPERS);
    s.boot();
    s.eval(
        "globalThis.room = new Node({ id: 'xr-room', position: { x: 1, y: 2, z: 3 }, rotation: { x: 0.1, y: 0.2, z: 0.3 }, scale: 2 });\n\
         room.metresPerUnit = 0.5;\n\
         rec('room', { id: room.id, rotation_y: room.rotation.y, rotation_1: room.rotation[1], scale: [...room.scale], mpu: room.metresPerUnit,\n\
           default_mpu: new Node({ id: 'plain' }).metresPerUnit, distance: room.position.distanceTo(new Node({ id: 'other', position: [1, 2, 7] }).position) });",
    )
    .step("made");
    s.tree("room", At::Named("xr-room"));
    s.eval(
        "globalThis.q = new Node({ id: 'q' }); q.rotation = { x: 0, y: Math.sin(0.25), z: 0, w: Math.cos(0.25) };\n\
         rec('quaternion', [...q.rotation]);\n\
         rec('bad', [fail(() => { q.rotation = { x: 0, y: NaN, z: 0 }; }), fail(() => { q.scale = 'big'; })]);",
    )
    .step("rotation_forms");
    s.tree("q", At::Named("q"));
    s.eval(
        "globalThis.n = new Node({ id: 'p', position: [1, 1, 1], rotation: [0, 0.5, 0] });\n\
         n.transform = { position: { x: 5, y: 6, z: 7 } };",
    )
    .step("transform_position_only");
    s.tree("p", At::Named("p"));
    s.eval(
        "rec('bad', [fail(() => { n.transform = { position: { x: 9, y: 9, z: 9 }, rotation: { x: 'no', y: 0, z: 0 } }; }),\n\
           fail(() => { n.transform = { position: [0, 0, 0], pose: 1 }; })]);\n\
         rec('read_back', { position: n.transform.position.toArray(), rotation: n.transform.rotation.toArray() });",
    )
    .step("transform_refused");
    s.tree("p", At::Named("p"));
    s.eval(
        "globalThis.parent = new Node({ id: 'parent', position: [10, 0, 0], rotation: [0, Math.PI / 2, 0], scale: 2 });\n\
         globalThis.child = new Node({ id: 'child', position: [0, 0, 1] }); parent.add(child);\n\
         const world = child.worldTransform;\n\
         rec('world', { position: world.position.toArray().map((x) => Math.round(x * 1e10) / 1e10), scale: world.scale });\n\
         rec('tree', { parent: child.parent === parent, children: parent.children.length, find: parent.find('child') === child });",
    )
    .step("parent_and_child");
    s.tree("parent", At::Named("parent"));
    s.eval("child.visible = false; child.opacity = 0.25; child.name = 'renamed'; child.input = 'none'; parent.input = 'capture';")
        .step("flags");
    s.tree("parent", At::Named("parent"));
    s.eval("parent.destroy(); rec('alive', [parent.alive, child.alive]);")
        .step("destroyed");
    s.finish();

    let mut s = Scn::new("scene_client_set", Home::Reference, &world(1), HELPERS);
    s.boot();
    s.eval(
        "globalThis.c = gnoblin.clients.at(0);\n\
         c.set({ opacity: 0.92, visible: true, transform: { rotation: { x: 0, y: 0.12, z: 0 } } }, { duration: 250, easing: 'ease-out' });",
    )
    .step("set_with_options");
    s.eval("rec('read', [...c.rotation]);").step("read_rotation");
    s.eval("c.set({ opacity: 1, transform: { position: { x: 30, y: 40, z: -5 }, scale: 1.5 } });")
        .step("set_atomic");
    s.eval("rec('read', { position: [c.position.x, c.position.y, c.position.z], committed: [c.committedSize.width, c.committedSize.height] });")
        .step("read_position_and_committed_size");
    s.eval("c.width = 400; rec('width', c.width);").step("asked_width");
    s.eval("rec('refused', fail(() => { c.set({ transform: { pose: 1 } }); }));")
        .step("refused_transform");
    s.finish();
}

#[test]
fn scene_root_and_views() {
    let mut s = Scn::new("scene_views", Home::Reference, &world(1), HELPERS);
    s.boot();
    s.eval(
        "rec('root', { instance: gnoblin.scene instanceof Scene, scene_new: fail(() => new Scene()), mpu: gnoblin.scene.metresPerUnit });\n\
         globalThis.room = new Node({ id: 'xr-room' }); gnoblin.scene.root.add(room);\n\
         rec('mount', { parent: room.parent === gnoblin.scene.root, listed: gnoblin.scene.root.children.includes(room), destroy_root: fail(() => gnoblin.scene.root.destroy()) });\n\
         gnoblin.scene.metresPerUnit = 0.001; rec('mpu', gnoblin.scene.metresPerUnit); rec('bad_mpu', fail(() => { gnoblin.scene.metresPerUnit = 0; }));",
    )
    .step("scene_root");
    s.tree("room", At::Named("xr-room"));
    s.eval(
        "globalThis.v = new View({ id: 'panel', content: gnoblin.clients.at(0), size: { width: 0.9, height: 0.55 }, contentFit: 'contain', input: 'forward' });\n\
         rec('input', v.input);",
    )
    .step("view_of_a_client");
    s.tree("panel", At::Named("panel"));
    for (mode, label) in [
        ("none", "input_none"),
        ("capture", "input_capture"),
        ("forward", "input_forward"),
    ] {
        s.eval(&format!("v.input = '{mode}'; rec('input', v.input);"))
            .step(label);
        s.tree("panel", At::Named("panel"));
    }
    s.eval("v.input = false; rec('input', v.input); v.input = true; rec('input', v.input); rec('bad', fail(() => { v.input = 'bogus'; }));")
        .step("input_booleans");
    s.eval(
        "gnoblin.requestTransition(() => { v.position = { x: 0.5, y: 0.2, z: -1.4 }; v.rotation = { x: 0, y: 0.3, z: 0 }; v.size = { width: 1.2, height: 0.75 }; }, { duration: 550, easing: 'ease-out' });\n\
         rec('view', { size: [v.size.width, v.size.height], fit: v.contentFit, content: v.content === gnoblin.clients.at(0) });\n\
         rec('bad', [fail(() => { v.contentFit = 'fill'; }), fail(() => { v.content = {}; })]);",
    )
    .step("view_transition");
    s.tree("panel", At::Named("panel"));
    s.finish();

    let mut s = Scn::new(
        "scene_workspace_view",
        Home::Reference,
        &world(2),
        "const desktop = new Workspace({ id: 'desktop', layout: new FloatingLayout() });\n\
         gnoblin.workspaces.add(desktop);\n",
    );
    s.boot();
    s.eval(
        "globalThis.overview = new Node({ id: 'overview' });\n\
         globalThis.big = overview.add(new View({ id: 'big', content: gnoblin.workspaces.get('desktop'), size: { width: 700, height: 400 } }));\n\
         globalThis.small = overview.add(new View({ id: 'small', content: gnoblin.workspaces.get('desktop'), size: { width: 350, height: 200 } }));",
    )
    .step("two_views_of_one_workspace");
    s.tree("big", At::Named("big"));
    s.tree("small", At::Named("small"));
    s.eval("rec('big', { same: big.content === gnoblin.workspaces.get('desktop'), orphaned: big.orphaned, children: big.children.length });")
        .step("read_back");
    s.eval("big.contentFit = 'contain';").step("contain");
    s.tree("big", At::Named("big"));
    s.eval("big.contentFit = 'cover';").step("cover");
    s.tree("big", At::Named("big"));
    s.open(900, "late", "late").step("window_joins");
    s.tree("big", At::Named("big"));
    s.destroy(3).step("window_leaves");
    s.tree("big", At::Named("big"));
    s.eval("big.destroy();").step("view_destroyed");
    s.eval("small.content = gnoblin.clients.at(0);")
        .step("view_given_a_client");
    s.tree("small", At::Named("small"));
    s.finish();
}

#[test]
fn cameras() {
    let mut s = Scn::new(
        "scene_cameras",
        Home::Reference,
        &world(1),
        &format!(
            "{HELPERS}globalThis.camera3d = (c: any) => ({{ projection: c.projection, metres: c.metres, near: c.near, far: c.far, fov: c.fov, ipd: c.ipd, size: c.size, budget: c.layerBudget }});\n\
             globalThis.isId = (c: any) => c.position.x === 0 && c.position.y === 0 && c.zoom === 1 && c.rotation === 0;\n"
        ),
    );
    s.boot();
    s.eval(
        "globalThis.cam = new Camera3D({ id: 'cam', projection: 'perspective', fov: 70, near: 0.2, position: { x: 1, y: 2, z: 3 } });\n\
         gnoblin.scene.root.add(cam); rec('camera', camera3d(cam));",
    )
    .step("perspective");
    s.tree("cam", At::Named("cam"));
    s.eval(
        "cam.projection = 'orthographic'; cam.size = 400; cam.far = 5000; cam.metres = 0.01; cam.ipd = 0.063; rec('camera', camera3d(cam));\n\
         rec('bad', [fail(() => { cam.projection = 'fisheye'; }), fail(() => { cam.size = 0; }), fail(() => { cam.fov = NaN; }), fail(() => { cam.near = -1; })]);",
    )
    .step("orthographic");
    s.tree("cam", At::Named("cam"));
    s.eval("cam.projection = 'stereo'; rec('camera', camera3d(cam));")
        .step("stereo");
    s.tree("cam", At::Named("cam"));
    s.eval(
        "globalThis.view = new View({ id: 'seen', content: gnoblin.clients.at(0), camera: cam, size: { width: 400, height: 300 } });\n\
         gnoblin.scene.root.add(view); rec('view', { camera: view.camera === cam, size: [view.size.width, view.size.height] });\n\
         view.camera = null; rec('camera', view.camera);",
    )
    .step("view_with_a_camera");
    s.tree("seen", At::Named("seen"));
    s.eval(
        "globalThis.flat = new Camera2D({ id: 'flat', position: { x: 10, y: 20 }, zoom: 2, rotation: 0.5 });\n\
         gnoblin.scene.root.add(flat);\n\
         rec('flat', { position: flat.position, zoom: flat.zoom, rotation: flat.rotation, identity: isId(flat) });\n\
         rec('bad', [fail(() => { flat.zoom = 0; }), fail(() => { flat.zoom = -2; }), fail(() => { flat.position = null; }), fail(() => { flat.position = { x: NaN, y: 0 }; })]);",
    )
    .step("flat_camera");
    s.tree("flat", At::Named("flat"));
    s.eval(
        "gnoblin.requestTransition(() => { flat.zoom = 4; flat.position = { x: 30, y: 40 }; flat.rotation = 1; cam.fov = 40; }, { duration: 100 });\n\
         rec('flat', { zoom: flat.zoom, fov: cam.fov });",
    )
    .step("camera_transition");
    s.tree("flat", At::Named("flat"));
    s.eval("flat.zoom = 1; flat.position = { x: 0, y: 0 }; flat.rotation = 0; rec('identity', isId(flat));")
        .step("back_to_identity");
    s.eval("cam.destroy(); flat.destroy(); rec('alive', [cam.alive, flat.alive]);")
        .step("destroyed");
    s.finish();
}

#[test]
fn bound_properties_of_a_node() {
    let mut s = Scn::new("scene_bindings", Home::Reference, &world(2), HELPERS);
    s.boot();
    s.eval(
        "globalThis.runs = 0; globalThis.c = gnoblin.clients.at(0);\n\
         globalThis.binding = c.bind('opacity', () => { runs++; return Math.max(0.25, 1 - c.x / 1000); });\n\
         rec('made', [runs, c.opacity]);",
    )
    .step("bound");
    s.settle_frames().eval("rec('runs', runs);").step("no_polling");
    s.set(3, "x", V::Num(200.0))
        .eval("rec('runs', runs);")
        .step("read_property_changes");
    s.set(3, "title", V::Text("renamed"))
        .set(3, "y", V::Num(50.0))
        .eval("rec('runs', runs);")
        .step("unrelated_changes");
    s.eval("c.x = 400;").eval("rec('runs', runs);").step("script_write");
    s.eval("binding.dispose(); rec('alive', binding.alive);")
        .set(3, "x", V::Num(900.0))
        .eval("rec('runs', runs);")
        .step("disposed");
    s.finish();

    let mut s = Scn::new(
        "scene_bindings_collections",
        Home::Reference,
        &world(2),
        "gnoblin.workspaces.add(new Workspace({ id: 'development', layout: new FloatingLayout() }));\n",
    );
    s.boot();
    s.eval("globalThis.ws = gnoblin.workspaces.get('development'); ws.properties.bind('name', () => (ws.clients.length > 2 ? 'busy' : 'quiet')); rec('name', ws.name);")
        .step("bound");
    s.open(30, "third", "third")
        .eval("rec('name', ws.name);")
        .step("window_joins");
    s.destroy(30).eval("rec('name', ws.name);").step("window_leaves");
    s.finish();

    let mut s = Scn::new("scene_bindings_failures", Home::Reference, &world(1), HELPERS);
    s.boot();
    s.eval(
        "globalThis.c = gnoblin.clients.at(0); globalThis.background = new Node({ id: 'background' }); globalThis.camera2D = { zoom: 2 };\n\
         background.properties.bind('vignette', () => Math.min(1, camera2D.zoom / 3));\n\
         c.properties.bind('opacity', () => { if (c.x > 100) throw new Error('too far'); return 0.5 + c.x / 1000; });",
    )
    .step("bound");
    s.set(3, "x", V::Num(300.0)).step("throws");
    s.set(3, "x", V::Num(400.0)).step("throws_again");
    s.set(3, "x", V::Num(10.0)).step("recovers");
    s.eval(
        "globalThis.a = new Node({ id: 'a' }); globalThis.b = new Node({ id: 'b' }); a.bind('x', () => b.x + 1); b.bind('x', () => a.x + 1);",
    )
    .step("feed_each_other");
    s.finish();

    let mut s = Scn::new("scene_bindings_owners", Home::Reference, &world(2), HELPERS);
    s.boot();
    s.eval(
        "globalThis.runs = 0; globalThis.n = new Node({ id: 'n' });\n\
         globalThis.first = n.bind('x', () => { runs++; return gnoblin.clients.at(0).x; });\n\
         globalThis.second = n.properties.bind('x', () => 7);\n\
         rec('replaced', [first.alive, second.alive, n.x]);\n\
         globalThis.c2 = gnoblin.clients.at(1); globalThis.bound = c2.bind('opacity', () => 0.4);",
    )
    .step("second_binding_replaces_the_first");
    s.destroy(4).eval("rec('alive', bound.alive);").step("owner_destroyed");
    s.eval("n.destroy(); rec('alive', second.alive); rec('refused', fail(() => { n.bind('x', () => 1); }));")
        .step("node_destroyed");
    s.finish();
}
