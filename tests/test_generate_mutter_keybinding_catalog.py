#!/usr/bin/env python3
"""Focused tests for the generated native shortcut descriptor format."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/generate-mutter-keybinding-catalog.py"
SPEC = importlib.util.spec_from_file_location("shortcut_catalog", SCRIPT)
assert SPEC and SPEC.loader
catalog_module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(catalog_module)


KEYBINDINGS_C = """
static BuiltinKeybinding COMMON_KEYBINDINGS[] = {
  { "close", 0, 0, handle_close, 0 },
  { "panel-run-dialog", 0, 0, NULL, 0 },
};
static BuiltinKeybinding MUTTER_KEYBINDINGS[] = {
  { "switch-monitor", 0, 0, handle_switch_monitor, 0 },
};
static BuiltinKeybinding WAYLAND_KEYBINDINGS[] = {
  { "restore-shortcuts", 0, 0, handle_restore_shortcuts, 0 },
};
static BuiltinKeybinding NATIVE_KEYBINDINGS[] = {
  { "switch-to-session-1", 0, 0, handle_switch_vt, 1 },
};
"""


def schema(schema_id: str, keys: str) -> str:
    return f"""<schemalist><schema id='{schema_id}'>{keys}</schema></schemalist>"""


def key(name: str, *, default: str = "[]", kind: str = "as", summary: str = "Action") -> str:
    return (
        f"<key name='{name}' type='{kind}'><default><![CDATA[{default}]]></default><summary>{summary}</summary></key>"
    )


class ShortcutCatalogTest(unittest.TestCase):
    def make_sources(self, root: Path, *, close_kind: str = "as", include_close: bool = True) -> tuple[Path, Path]:
        mutter = root / "mutter"
        schemas = root / "gsettings-desktop-schemas"
        (mutter / "src/core").mkdir(parents=True)
        (mutter / "data").mkdir()
        (schemas / "schemas").mkdir(parents=True)
        (mutter / "src/core/keybindings.c").write_text(KEYBINDINGS_C, encoding="utf-8")
        close_key = key("close", default="['<Alt>F4']", kind=close_kind, summary="Close window")
        if not include_close:
            close_key = ""
        (schemas / "schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in").write_text(
            schema(
                "org.gnome.desktop.wm.keybindings",
                close_key + key("panel-run-dialog", summary="Run prompt"),
            ),
            encoding="utf-8",
        )
        (mutter / "data/org.gnome.mutter.gschema.xml.in").write_text(
            schema("org.gnome.mutter.keybindings", key("switch-monitor", summary="Switch monitors")),
            encoding="utf-8",
        )
        (mutter / "data/org.gnome.mutter.wayland.gschema.xml.in").write_text(
            schema(
                "org.gnome.mutter.wayland.keybindings",
                key("restore-shortcuts", default="['<Super>Escape']", summary="Restore shortcuts")
                + key("switch-to-session-1", summary="Switch to VT 1"),
            ),
            encoding="utf-8",
        )
        return mutter, schemas

    def test_emits_only_handler_backed_descriptors_with_schema_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            mutter, schemas = self.make_sources(Path(temporary))
            result = catalog_module.extract_catalog(mutter, schemas)

        self.assertEqual(result["format"], 2)
        self.assertEqual(
            result["groups"]["wm"]["close"],
            {
                "key": "close",
                "description": "Close window",
                "default_bindings": ["<Alt>F4"],
            },
        )
        self.assertNotIn("panel-run-dialog", result["groups"]["wm"])
        self.assertEqual(
            result["groups"]["wayland"]["switch-to-session-1"]["default_bindings"],
            [],
        )

    def test_fails_if_executable_action_is_missing_from_its_pinned_schema(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            mutter, schemas = self.make_sources(Path(temporary), include_close=False)
            with self.assertRaisesRegex(ValueError, "no schema key: wm.close"):
                catalog_module.extract_catalog(mutter, schemas)

    def test_fails_if_executable_action_schema_key_is_not_string_array(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            mutter, schemas = self.make_sources(Path(temporary), close_kind="s")
            with self.assertRaisesRegex(ValueError, "not a string-array key: wm.close"):
                catalog_module.extract_catalog(mutter, schemas)

    def test_fails_on_unparseable_string_array_default(self) -> None:
        with self.assertRaisesRegex(ValueError, "unsupported string-array default syntax"):
            catalog_module.parse_string_array_default("['ok', invalid]", location="wm.close")
        with self.assertRaisesRegex(ValueError, "unsupported string-array default syntax"):
            catalog_module.parse_string_array_default("['ok',, 'extra']", location="wm.close")


if __name__ == "__main__":
    unittest.main()
