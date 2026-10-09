#!/usr/bin/env python3
"""Regression checks for native-control snapshot conversion ownership."""

from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _sources import control_source  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]


def extract_function(source: str, signature: str) -> str:
    """Return a complete C function without maintaining a second copy in tests."""
    start = source.index(signature)
    body_start = source.index("{", start)
    depth = 0
    for index in range(body_start, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    raise AssertionError(f"unterminated function: {signature}")


class NativeControlOwnershipTests(unittest.TestCase):
    def test_variant_from_json_releases_nested_variants(self):
        converter = extract_function(control_source(), "static GVariant* variant_from_json(")
        harness = textwrap.dedent(
            f"""
            #include <glib.h>
            #include <json-glib/json-glib.h>
            #include <malloc.h>
            #include <stdio.h>

            {converter}

            int main(void) {{
                g_autoptr(JsonParser) parser = json_parser_new();
                g_assert_true(json_parser_load_from_data(
                    parser,
                    "{{\\\"active\\\":true,\\\"windows\\\":[{{\\\"title\\\":\\\"one\\\"}},{{\\\"title\\\":\\\"two\\\"}}]}}",
                    -1, NULL));

                for (guint i = 0; i < 100; i++) {{
                    g_autoptr(GVariant) warmup = variant_from_json(json_parser_get_root(parser));
                    g_assert_nonnull(warmup);
                }}
                malloc_trim(0);
                struct mallinfo2 before = mallinfo2();
                for (guint i = 0; i < 10000; i++) {{
                    g_autoptr(GVariant) value = variant_from_json(json_parser_get_root(parser));
                    g_assert_nonnull(value);
                }}
                malloc_trim(0);
                struct mallinfo2 after = mallinfo2();
                if (after.uordblks - before.uordblks > 1024 * 1024) {{
                    fprintf(stderr, "nested conversion retained %td bytes\\n",
                            after.uordblks - before.uordblks);
                    return 1;
                }}
                return 0;
            }}
            """
        )
        with tempfile.TemporaryDirectory(prefix="gnoblin-native-control-ownership-") as directory:
            work = Path(directory)
            source = work / "ownership.c"
            executable = work / "ownership"
            source.write_text(harness)
            flags = subprocess.check_output(
                ["pkg-config", "--cflags", "--libs", "glib-2.0", "json-glib-1.0"], text=True
            ).split()
            subprocess.run(
                ["cc", "-std=c11", "-O2", "-Wall", "-Werror", str(source), "-o", str(executable), *flags],
                check=True,
            )
            subprocess.run([str(executable)], check=True)

    def test_window_state_comparison_excludes_transport_revision(self):
        source = control_source()
        function = extract_function(source, "static void publish_window_changes(")
        comparable = function[function.index("g_autoptr(JsonNode) comparable") : function.index("state->comparable_json")]
        self.assertIn('json_object_remove_member(json_node_get_object(comparable), "focused");', comparable)
        self.assertIn('json_object_remove_member(json_node_get_object(comparable), "revision");', comparable)


if __name__ == "__main__":
    unittest.main()
