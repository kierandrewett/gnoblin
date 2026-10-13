#!/usr/bin/env python3
"""Private synthetic procfs checks; never scan the host or open a DRM device.

Run: python3 tests/incident-resources.test.py
"""

import importlib.util
from contextlib import contextmanager
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/gnoblin-incident-resources.py"
spec = importlib.util.spec_from_file_location("incident_resources", SCRIPT)
recorder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recorder)

FDINFO = """pos: 0
drm-driver: amdgpu
drm-pdev: 0000:01:00.0
drm-client-id: 7
drm-resident-gtt: 2 MiB
drm-memory-gtt: 2048 KiB
drm-total-gtt: 4194304
drm-shared-gtt: 1024 KiB
drm-engine-gfx: 123 ns
drm-client-name: PRIVATE-CLIENT-NAME
driver-private: PRIVATE-PAYLOAD
"""


class RecorderTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="gnoblin-incident-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.proc = self.root / "proc"
        self.proc.mkdir()
        (self.proc / "meminfo").write_text("GPUActive: 2048 kB\nSwapFree: 4096 kB\n")

    def process(self, pid, data=FDINFO, start=100):
        p = self.proc / str(pid)
        (p / "fdinfo").mkdir(parents=True, exist_ok=True)
        (p / "fd").mkdir(exist_ok=True)
        if not (p / "fd/9").exists():
            (p / "fd/9").symlink_to("/dev/null")
        # Fields 3..22: names may contain spaces and closing parentheses.
        fields = ["S", "1"] + ["0"] * 17 + [str(start)]
        (p / "stat").write_text(f"{pid} (fixture ) app) " + " ".join(fields))
        (p / "fdinfo/9").write_text(data)
        (p / "cmdline").write_text("PRIVATE-ARGUMENT")
        return p

    def snapshot(self, **kwargs):
        return recorder.snapshot(self.proc, **kwargs)

    def test_shared_client_keeps_all_holders_without_summing_duplicate_fds(self):
        p = self.process(101)
        (p / "fdinfo/10").write_text(FDINFO)
        (p / "fd/10").symlink_to("/dev/null")
        self.process(202, start=200)
        row = self.snapshot()
        self.assertEqual(len(row["gpuClients"]), 1)
        c = row["gpuClients"][0]
        self.assertEqual(c["memoryBytes"]["drm-resident-gtt"], 2097152)
        self.assertNotIn("drm-memory-gtt", c["memoryBytes"])
        self.assertEqual({(h["pid"], h["startTicks"]) for h in c["holders"]}, {(101, 100), (202, 200)})
        self.assertEqual(sum(h["fdCount"] for h in c["holders"]), 3)
        self.assertNotIn("PRIVATE", json.dumps(row))
        self.assertEqual(row["coverage"]["errors"], {})
        self.assertEqual(row["coverage"]["limitsHit"], [])

    def test_device_scope_and_missing_optional_pdev(self):
        self.process(101)
        self.process(202, FDINFO.replace("0000:01:00.0", "0000:02:00.0"))
        self.process(303, FDINFO.replace("drm-pdev: 0000:01:00.0\n", ""))
        self.assertEqual(len(self.snapshot()["gpuClients"]), 3)

    def test_missing_pdev_never_merges_possible_devices_or_duplicate_fds(self):
        data = FDINFO.replace("drm-pdev: 0000:01:00.0\n", "")
        p = self.process(101, data)
        (p / "fdinfo/10").write_text(data)
        (p / "fd/10").symlink_to("/dev/null")
        self.process(202, data)
        row = self.snapshot()
        # Identical driver/client IDs could denote different non-PCI devices.
        # Without verified device identity even duplicated fds remain separate.
        self.assertEqual(len(row["gpuClients"]), 3)
        self.assertEqual(row["coverage"]["errors"]["ambiguousDeviceFds"], 3)
        self.assertEqual(
            {(c["holders"][0]["pid"], c["observedFd"]) for c in row["gpuClients"]},
            {(101, 9), (101, 10), (202, 9)},
        )
        for client in row["gpuClients"]:
            self.assertEqual(client["identityScope"], "fd")
            self.assertEqual(client["holders"][0]["fdCount"], 1)
            self.assertEqual(client["memoryBytes"]["drm-resident-gtt"], 2097152)

    def test_legacy_memory_alias_and_invalid_units(self):
        self.process(
            101,
            FDINFO.replace("drm-resident-gtt: 2 MiB\n", "") + "drm-resident-vram: -1 KiB\ndrm-total-cpu: 4 bananas\n",
        )
        row = self.snapshot()
        self.assertEqual(row["gpuClients"][0]["memoryBytes"]["drm-resident-gtt"], 2097152)
        self.assertEqual(row["coverage"]["errors"]["invalidDrmFields"], 2)

    def test_only_character_devices_are_candidates_without_path_or_major_filter(self):
        p = self.process(101)
        # A nonstandard alias of a non-DRM-major character device still qualifies.
        alias = self.root / "arbitrary-device-name"
        alias.symlink_to("/dev/null")
        (p / "fd/9").unlink()
        (p / "fd/9").symlink_to(alias)
        (p / "fdinfo/10").write_text("x" * 20000)
        regular = self.root / "not-a-device"
        regular.write_text("PRIVATE FILE DATA")
        (p / "fd/10").symlink_to(regular)
        original = recorder.read_bounded

        def read(path, *args):
            self.assertNotEqual(path, p / "fdinfo/10", "non-device fdinfo must not be read")
            return original(path, *args)

        with patch.object(recorder, "read_bounded", side_effect=read):
            row = self.snapshot()
        self.assertEqual(len(row["gpuClients"]), 1)
        self.assertEqual(row["coverage"]["fdEntries"], 2)
        self.assertEqual(row["coverage"]["nonDeviceFds"], 1)
        self.assertEqual(row["coverage"]["fdinfoFiles"], 1)
        self.assertTrue(row["coverage"]["traversalComplete"])
        self.assertEqual(row["coverage"]["errors"], {})

    def test_unavailable_fd_target_falls_back_to_fdinfo_with_visible_gap(self):
        p = self.process(101)
        (p / "fd/9").unlink()
        row = self.snapshot()
        self.assertEqual(len(row["gpuClients"]), 1)
        self.assertEqual(row["coverage"]["errors"]["fdTargetUnavailable"], 1)

    def test_fd_entry_budget_is_separate_from_fdinfo_read_budget(self):
        p = self.process(101)
        (p / "fdinfo/10").write_text(FDINFO)
        (p / "fd/10").symlink_to("/dev/null")
        row = self.snapshot(max_fd_entries=1)
        self.assertEqual(row["coverage"]["fdEntries"], 1)
        self.assertIn("fdEntries", row["coverage"]["limitsHit"])
        self.assertFalse(row["coverage"]["traversalComplete"])

    def test_later_process_survives_earlier_non_device_fds_without_spending_reads(self):
        early = self.process(101, "pos: 0\n")
        (early / "fd/9").unlink()
        (early / "fd/9").symlink_to(early / "cmdline")
        self.process(202)
        original = recorder.os.scandir

        @contextmanager
        def ordered(path):
            with original(path) as entries:
                yield iter(sorted(entries, key=lambda entry: entry.name))

        # Force the early non-device process before the late DRM owner. A single
        # fdinfo read is enough only if non-device fds consume no read budget.
        with patch.object(recorder.os, "scandir", side_effect=ordered):
            row = self.snapshot(max_fds=1)
        self.assertEqual([h["pid"] for h in row["gpuClients"][0]["holders"]], [202])
        self.assertEqual(row["coverage"]["fdinfoFiles"], 1)
        self.assertEqual(row["coverage"]["nonDeviceFds"], 1)

    def test_denied_target_metadata_does_not_hide_readable_fdinfo(self):
        p = self.process(101)
        original = recorder.os.stat

        def denied(path, *args, **kwargs):
            if str(path) == str(p / "fd/9"):
                raise PermissionError()
            return original(path, *args, **kwargs)

        with patch.object(recorder.os, "stat", side_effect=denied):
            row = self.snapshot()
        self.assertEqual(len(row["gpuClients"]), 1)
        self.assertEqual(row["coverage"]["errors"]["fdTargetUnavailable"], 1)

    def test_missing_client_id_is_reported_without_inventing_identity(self):
        self.process(101, FDINFO.replace("drm-client-id: 7\n", ""))
        row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["coverage"]["errors"]["unidentifiedDrmFds"], 1)

    def test_invalid_device_identity_cannot_merge_into_a_global_client(self):
        self.process(101, FDINFO.replace("0000:01:00.0", "invalid-device"))
        self.process(202, FDINFO.replace("drm-pdev: 0000:01:00.0\n", ""))
        row = self.snapshot()
        self.assertEqual(len(row["gpuClients"]), 1)
        self.assertEqual([h["pid"] for h in row["gpuClients"][0]["holders"]], [202])
        self.assertEqual(row["coverage"]["errors"]["invalidDrmFields"], 1)

    def test_non_drm_is_ignored_but_malformed_drm_is_visible(self):
        self.process(101, "pos: 0\nflags: 02000002\n")
        self.process(202, FDINFO.replace("drm-driver: amdgpu\n", ""))
        row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["coverage"]["errors"]["unidentifiedDrmFds"], 1)

    def test_counter_limit_marks_incomplete_usage_without_losing_holder(self):
        data = "drm-driver: amdgpu\ndrm-client-id: 7\n"
        data += "".join(f"drm-resident-region{i}: 0\n" for i in range(65))
        self.process(101, data)
        row = self.snapshot()
        self.assertEqual(len(row["gpuClients"][0]["memoryBytes"]), 64)
        self.assertEqual(row["coverage"]["errors"]["counterLimit"], 1)

    def test_denied_and_disappearing_fdinfo_are_distinct_from_zero_usage(self):
        self.process(101)
        self.process(202)
        original = recorder.read_bounded

        def read(path, *args):
            if path == self.proc / "101/fdinfo/9":
                raise PermissionError()
            if path == self.proc / "202/fdinfo/9":
                raise FileNotFoundError()
            return original(path, *args)

        with patch.object(recorder, "read_bounded", side_effect=read):
            row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["coverage"]["errors"]["permissionDenied"], 1)
        self.assertEqual(row["coverage"]["errors"]["vanished"], 1)

    def test_process_reuse_during_scan_discards_misattributed_clients(self):
        self.process(101)
        original = recorder.read_bounded
        reads = 0

        def read(path, *args):
            nonlocal reads
            text = original(path, *args)
            if path.name == "stat":
                reads += 1
                if reads == 2:
                    return text.rsplit(" ", 1)[0] + " 999"
            return text

        with patch.object(recorder, "read_bounded", side_effect=read):
            row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["coverage"]["errors"]["processChanged"], 1)

    def test_fd_process_client_holder_and_time_budgets_are_visible(self):
        self.process(101)
        self.process(202)
        for kwargs, flag in (
            ({"max_fds": 1}, "fds"),
            ({"max_processes": 1}, "processes"),
            ({"max_holders": 1}, "holders"),
            ({"budget_seconds": 0}, "time"),
        ):
            with self.subTest(flag=flag):
                self.assertIn(flag, self.snapshot(**kwargs)["coverage"]["limitsHit"])
        self.process(303, FDINFO.replace("drm-client-id: 7", "drm-client-id: 8"))
        self.assertIn("clients", self.snapshot(max_clients=1)["coverage"]["limitsHit"])

    def test_oversized_input_is_not_parsed_as_complete(self):
        self.process(101, FDINFO + "x" * 20000)
        row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["coverage"]["errors"]["oversizedFiles"], 1)

    def test_missing_meminfo_is_explicit_and_no_stale_holders_survive(self):
        self.process(101)
        self.assertEqual(len(self.snapshot()["gpuClients"]), 1)
        (self.proc / "101/fdinfo/9").unlink()
        (self.proc / "meminfo").unlink()
        row = self.snapshot()
        self.assertEqual(row["gpuClients"], [])
        self.assertEqual(row["memKiB"], {})
        self.assertEqual(row["coverage"]["errors"]["vanished"], 1)

    def test_private_rotation_bound_and_exclusive_writer_cleanup(self):
        out = self.root / "output"
        with recorder.RecordWriter(out, max_bytes=180, backups=3) as writer:
            for i in range(20):
                writer.write({"sequence": i, "payload": "x" * 70})
            with self.assertRaises((BlockingIOError, RuntimeError)):
                with recorder.RecordWriter(out):
                    pass
            with self.assertRaises(ValueError):
                writer.write({"oversize": "x" * 200})
        files = list(out.glob("gpu-resources*.jsonl"))
        self.assertEqual(len(files), 4)
        self.assertLessEqual(sum(f.stat().st_size for f in files), 4 * 180)
        for f in files:
            self.assertEqual(f.stat().st_mode & 0o777, 0o600)
            for line in f.read_text().splitlines():
                json.loads(line)
        with recorder.RecordWriter(out):
            pass

    def test_writer_rejects_symlinks_and_public_directory(self):
        out = self.root / "output"
        out.mkdir(mode=0o700)
        target = self.root / "untouched"
        target.write_text("keep")
        (out / "gpu-resources.jsonl").symlink_to(target)
        with self.assertRaises((OSError, ValueError)):
            with recorder.RecordWriter(out) as writer:
                writer.write({"x": 1})
        self.assertEqual(target.read_text(), "keep")
        out.chmod(0o755)
        with self.assertRaises((OSError, ValueError)):
            with recorder.RecordWriter(out):
                pass

    def test_cli_sigterm_closes_writer_during_interval(self):
        self.process(101)
        out = self.root / "output"
        command = [
            sys.executable,
            str(SCRIPT),
            "--proc-root",
            str(self.proc),
            "--output",
            str(out),
            "--samples",
            "2",
            "--interval",
            "10",
        ]
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.addCleanup(lambda: child.poll() is None and child.kill())
        deadline = time.monotonic() + 5
        log = out / "gpu-resources.jsonl"
        while time.monotonic() < deadline:
            if log.exists() and log.stat().st_size:
                break
            if child.poll() is not None:
                self.fail(str(child.communicate()))
            time.sleep(0.01)
        else:
            self.fail("synthetic recorder did not produce its first row")
        child.send_signal(signal.SIGTERM)
        stdout, stderr = child.communicate(timeout=2)
        self.assertEqual((child.returncode, stdout, stderr), (0, b"", b""))
        self.assertEqual(len(log.read_text().splitlines()), 1)
        with recorder.RecordWriter(out):
            pass


if __name__ == "__main__":
    unittest.main()
