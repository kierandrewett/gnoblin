#!/usr/bin/env python3
"""Exercise the shared quality commands in a disposable Git repository."""

from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class QualityToolsTest(unittest.TestCase):
    def test_check_format_and_failure_preservation(self):
        with tempfile.TemporaryDirectory(prefix="repo-quality-") as directory:
            root = Path(directory)
            for name in (
                ".pre-commit-config.yaml",
                ".prettierrc.json",
                ".prettierignore",
                ".clang-format",
                ".stylua.toml",
                "ruff.toml",
                "eslint.config.mjs",
                "scripts/quality.sh",
                "scripts/format-qt.py",
            ):
                destination = root / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, destination)
            samples = {
                "sample.py": "value=  1\n",
                "without-extension": "#!/usr/bin/env python3\nvalue=  1\n",
                "sample.c": "int main(void){return 0;}\n",
                "sample.sh": '#!/usr/bin/env bash\nset -euo pipefail\nif true; then echo "ok"; fi\n',
                "sample.js": '.pragma library\n.import "Other.js" as Other\nfunction value(){return 1}\n',
                "sample.mjs": "export const value={a:1}\n",
                "sample.qml": "import QtQuick\nQtObject { property int value: 1 }\n",
                "sample.lua": "local value={a=1}\nreturn value\n",
                "sample.rs": 'fn main(){println!("ok");}\n',
                "sample.nix": "{value=1;}\n",
                "file with spaces.json": '{"value":1}\n',
                "build/ignored.py": "deliberately invalid Python (\n",
            }
            for name, content in samples.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
                if content.startswith("#!"):
                    path.chmod(0o755)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            def run(mode):
                return subprocess.run(
                    [str(root / "scripts/quality.sh"), mode], cwd=root, capture_output=True, text=True
                )

            initial = {name: (root / name).read_bytes() for name in samples}
            result = run("lint")
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(initial, {name: (root / name).read_bytes() for name in samples})
            result = run("format")
            # pre-commit returns 1 when formatters modify tracked files.
            self.assertIn(result.returncode, (0, 1), result.stdout + result.stderr)
            result = run("lint")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((root / "sample.js").read_text().startswith(samples["sample.js"].split("function")[0]))
            self.assertNotEqual(initial["without-extension"], (root / "without-extension").read_bytes())
            self.assertEqual(initial["build/ignored.py"], (root / "build/ignored.py").read_bytes())
            result = run("format")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

            # A parser failure must not replace the file with partial output.
            broken = root / "sample.qml"
            broken.write_text("import QtQuick\nQtObject { property int value: (\n")
            before = broken.read_bytes()
            result = subprocess.run(
                ["python3", "scripts/format-qt.py", "--write", "sample.qml"], cwd=root, capture_output=True
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(before, broken.read_bytes())


if __name__ == "__main__":
    unittest.main()
