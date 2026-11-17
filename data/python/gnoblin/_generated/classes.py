"""Stub until `cargo xtask gen-sdks` writes the real file (owner: SdkGen). Do not extend by hand."""

from __future__ import annotations

from .._base import Method, Node, Property

CATALOG_HASH = 0


class Window(Node):
    __class_name__ = "window"
    title = Property("title", str, readonly=True)
    app_id = Property("app_id", str, readonly=True)
    focused = Property("focused", bool, readonly=True)
    x = Property("x", float, animated=True)
    y = Property("y", float, animated=True)
    width = Property("width", float, animated=True)
    height = Property("height", float, animated=True)
    maximised = Property("maximised", bool)
    focus = Method("focus", (), None)
    close = Method("close", (), None)
    kill = Method("kill", (), None)


class Seat(Node):
    __class_name__ = "seat"
    focus = Property("focus", "Window | None", readonly=True)


class Screen(Node):
    __class_name__ = "screen"
    screen_x = Property("screen_x", float, readonly=True)
    screen_y = Property("screen_y", float, readonly=True)
    screen_width = Property("screen_width", float, readonly=True)
    screen_height = Property("screen_height", float, readonly=True)


__all__ = ["CATALOG_HASH", "Screen", "Seat", "Window"]
