import assert from "node:assert/strict";
import test from "node:test";

import {
    TouchpadGestureRouter,
    touchpadGesturePathMatches,
} from "../src/gnome-shell-overlay/js/ui/components/gnoblinTouchpadGestureCore.js";

const rightThenDown = [
    { x: 0, y: 0 },
    { x: 1, y: 0 },
    { x: 1, y: 1 },
];

test("matches the same path with different scale and event sampling", () => {
    assert.equal(
        touchpadGesturePathMatches(rightThenDown, [
            { x: 0, y: 0 },
            { x: 12, y: 0 },
            { x: 25, y: 0 },
            { x: 25, y: 10 },
            { x: 25, y: 25 },
        ]),
        true,
    );
});

test("does not treat a different direction or turn order as the configured path", () => {
    assert.equal(
        touchpadGesturePathMatches(rightThenDown, [
            { x: 0, y: 0 },
            { x: 0, y: 1 },
            { x: 1, y: 1 },
        ]),
        false,
    );
    assert.equal(
        touchpadGesturePathMatches(rightThenDown, [
            { x: 0, y: 0 },
            { x: -1, y: 0 },
            { x: -1, y: 1 },
        ]),
        false,
    );
});

test("honors tolerance and rejects degenerate paths", () => {
    const noisy = [
        { x: 0, y: 0 },
        { x: 1, y: 0.08 },
        { x: 1.06, y: 1 },
    ];
    assert.equal(touchpadGesturePathMatches(rightThenDown, noisy, 0.1), true);
    assert.equal(touchpadGesturePathMatches(rightThenDown, noisy, 0.01), false);
    assert.equal(
        touchpadGesturePathMatches(rightThenDown, [
            { x: 0, y: 0 },
            { x: 0, y: 0 },
        ]),
        false,
    );
    assert.equal(touchpadGesturePathMatches(null, rightThenDown), false);
});

function makeRouter(gestures, context = "normal") {
    const fired = [];
    const router = new TouchpadGestureRouter({
        getContexts: () => [context],
        runGesture: (gesture) => fired.push(gesture.name),
    });
    router.configure(gestures);
    return { router, fired };
}

function swipe(router, points, fingers = 3) {
    const claimed = [router.handle({ gesture: "swipe", phase: "begin", fingers })];
    for (const [dx, dy] of points) claimed.push(router.handle({ gesture: "swipe", phase: "update", fingers, dx, dy }));
    claimed.push(router.handle({ gesture: "swipe", phase: "end", fingers }));
    return claimed;
}

test("routes a matching swipe once, and consumes a claimed but unmatched path", () => {
    const { router, fired } = makeRouter([
        {
            name: "right-down",
            gesture: "swipe",
            fingers: 3,
            path: rightThenDown,
            threshold: 16,
            tolerance: 0.2,
            when: "normal",
            command: ["true"],
        },
    ]);

    assert.deepEqual(
        swipe(router, [
            [20, 0],
            [0, 20],
        ]),
        [true, true, true, true],
    );
    assert.deepEqual(fired, ["right-down"]);
    assert.deepEqual(
        swipe(router, [
            [0, 20],
            [20, 0],
        ]),
        [true, true, true, true],
    );
    assert.deepEqual(fired, ["right-down"]);
});

test("passes through gestures with no matching finger count or context", () => {
    const { router, fired } = makeRouter([
        {
            name: "app-grid",
            gesture: "swipe",
            fingers: 4,
            path: rightThenDown,
            threshold: 16,
            tolerance: 0.2,
            when: "app-grid",
            action: "app-grid.show",
        },
    ]);

    assert.deepEqual(
        swipe(
            router,
            [
                [20, 0],
                [0, 20],
            ],
            3,
        ),
        [false, false, false, false],
    );
    assert.deepEqual(fired, []);
});

test("does not take over Shell progress gestures", () => {
    const { router, fired } = makeRouter([
        {
            name: "system-workspace-progress",
            gesture: "swipe",
            fingers: 3,
            path: rightThenDown,
            threshold: 16,
            tolerance: 0.2,
            when: "any",
            action: "workspace.progress",
        },
    ]);

    assert.deepEqual(
        swipe(router, [
            [20, 0],
            [0, 20],
        ]),
        [false, false, false, false],
    );
    assert.deepEqual(fired, []);
});

test("recognizes pinch direction and cancels without firing", () => {
    const { router, fired } = makeRouter([
        {
            name: "zoom-out",
            gesture: "pinch",
            fingers: 2,
            direction: "out",
            threshold: 0.1,
            when: "normal",
            command: ["true"],
        },
    ]);

    assert.equal(router.handle({ gesture: "pinch", phase: "begin", fingers: 2, scale: 1 }), true);
    assert.equal(router.handle({ gesture: "pinch", phase: "end", fingers: 2, scale: 1.2 }), true);
    assert.deepEqual(fired, ["zoom-out"]);
    assert.equal(router.handle({ gesture: "pinch", phase: "begin", fingers: 2, scale: 1 }), true);
    assert.equal(router.handle({ gesture: "pinch", phase: "cancel", fingers: 2, scale: 1.2 }), true);
    assert.deepEqual(fired, ["zoom-out"]);
});
