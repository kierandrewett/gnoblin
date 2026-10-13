#!/usr/bin/env python3
"""Local regression for the real pressure log reader and result aggregation.

No client, compositor, VM, input session, or load unit is started.
"""

import importlib.util
import io
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("guest_pressure", Path(__file__).with_name("guest-pressure.py"))
pressure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pressure)

START = 9_000_000


def read_records(lines):
    client = pressure.ClientLog.__new__(pressure.ClientLog)
    client.records = []
    client.policy_error = None
    client.pid = 123
    client.log = io.StringIO()
    client.process = SimpleNamespace(stdout=io.StringIO(f"START {START}\n{lines}"))
    with patch.object(pressure, "process_policy", return_value={"nice": 0, "scheduler": 0}):
        client._read()
    return client.records


def phase(name, start, end):
    return {
        "name": name,
        "start_absolute_us": START + start,
        "end_absolute_us": START + end,
        "start_client_us": start,
        "end_client_us": end,
    }


def sent(name, client_us):
    return {"phase": name, "sent_absolute_us": START + client_us, "sent_client_us": client_us}


class PressureEvidenceTests(unittest.TestCase):
    def summarize(self, lines, phases=None, injections=None, rows=None, completed=True):
        phases = phases if phases is not None else [phase("combined", 100_000, 1_000_000)]
        injections = injections if injections is not None else [sent("combined", 110_000)]
        rows = rows if rows is not None else [{"phase": "combined", "dbus_rtt_ms": 3.25}]
        result = {"lifecycle_passed": completed, "errors": []}
        inputs = pressure.summarize_observations(result, rows, phases, injections, read_records(lines))
        return result, phases, inputs

    def test_slow_cadence_can_only_pass_lifecycle(self):
        result, phases, inputs = self.summarize(
            "FRAME 50000 0 4294967290\n"
            "FRAME 500000 450000 3\n"
            "FRAME 950000 450000 4\n"
            f"KEY {START + 230_000} 30 1\n"
            f"KEY {START + 230_001} 30 0\n"
            f"KEY {START + 230_002} 31 1\n"
        )
        self.assertTrue(result["lifecycle_passed"])
        self.assertNotIn("passed", result)
        self.assertEqual(result["performance"]["status"], "not_evaluated")
        self.assertEqual(result["errors"], [])
        self.assertEqual(phases[0]["frame_p95_ms"], 450)
        self.assertEqual(phases[0]["frame_max_ms"], 450)
        self.assertEqual(phases[0]["frame_over_100ms"], 2)
        self.assertEqual(phases[0]["frame_cross_boundary_intervals"], 1)
        self.assertEqual(phases[0]["input_p95_ms"], 120)
        self.assertEqual(phases[0]["dbus_p95_ms"], 3.25)
        self.assertEqual(inputs["paired"][0]["received_absolute_us"], START + 230_000)

    def test_first_callback_is_not_a_zero_duration_interval(self):
        result, phases, _ = self.summarize(f"FRAME 120000 0 1\nKEY {START + 130_000} 30 1\n")
        self.assertFalse(result["lifecycle_passed"])
        self.assertEqual(phases[0]["frame_callbacks"], 1)
        self.assertEqual(phases[0]["frame_intervals"], 0)
        self.assertIsNone(phases[0]["frame_p95_ms"])
        self.assertIsNone(phases[0]["frame_max_ms"])

    def test_phase_end_is_exclusive_and_delayed_input_belongs_to_send_phase(self):
        result, phases, _ = self.summarize(
            "FRAME 50000 0 1\nFRAME 150000 100000 2\n"
            "FRAME 200000 50000 3\nFRAME 320000 120000 4\n"
            f"KEY {START + 210_000} 30 1\nKEY {START + 230_000} 30 1\n",
            phases=[phase("baseline", 100_000, 200_000), phase("combined", 200_000, 320_000)],
            injections=[sent("baseline", 190_000), sent("combined", 220_000)],
            rows=[{"phase": "baseline", "dbus_rtt_ms": 2}, {"phase": "combined", "dbus_rtt_ms": 4}],
        )
        self.assertTrue(result["lifecycle_passed"])
        self.assertEqual([p["frame_callbacks"] for p in phases], [1, 1])
        self.assertEqual([p["frame_p95_ms"] for p in phases], [100, 50])
        self.assertEqual([p["frame_cross_boundary_intervals"] for p in phases], [1, 1])
        self.assertEqual([p["input_p95_ms"] for p in phases], [20, 10])
        self.assertEqual(phases[0]["frame_over_100ms"], 0)

    def test_percentiles_use_nearest_rank_and_convert_microseconds_once(self):
        # Twenty deliberately unsorted intervals: p95 is 19 ms, max is 20 ms.
        durations = [20, *range(1, 20)]
        lines = ["FRAME 100000 0 1"]
        timestamp = 100_000
        injections, rows = [], []
        for index, duration in enumerate(durations):
            timestamp += duration * 1000
            lines.append(f"FRAME {timestamp} {duration * 1000} {index + 2}")
            key_sent = 110_000 + index * 30_000
            injections.append(sent("combined", key_sent))
            lines.append(f"KEY {START + key_sent + duration * 1000} 30 1")
            rows.append({"phase": "combined", "dbus_rtt_ms": duration / 2})
        result, phases, _ = self.summarize("\n".join(lines), injections=injections, rows=rows)
        self.assertTrue(result["lifecycle_passed"])
        self.assertEqual(phases[0]["frame_callbacks"], 21)
        self.assertEqual(phases[0]["frame_intervals"], 20)
        self.assertEqual(phases[0]["frame_p95_ms"], 19)
        self.assertEqual(phases[0]["frame_max_ms"], 20)
        self.assertEqual(phases[0]["input_p95_ms"], 19)
        self.assertEqual(phases[0]["input_max_ms"], 20)
        self.assertEqual(phases[0]["dbus_p95_ms"], 9.5)

    def test_missing_extra_or_early_keys_do_not_publish_shifted_latencies(self):
        for receipts in ([130_000], [130_000, 230_000, 240_000], [100_000, 230_000], [230_000, 220_000]):
            with self.subTest(receipts=receipts):
                result, phases, inputs = self.summarize(
                    "FRAME 50000 0 1\nFRAME 150000 100000 2\n" + "".join(f"KEY {START + t} 30 1\n" for t in receipts),
                    injections=[sent("combined", 110_000), sent("combined", 210_000)],
                )
                self.assertFalse(result["lifecycle_passed"])
                self.assertEqual(inputs["paired"], [])
                self.assertEqual(len(inputs["received"]), len(receipts))
                self.assertIsNone(phases[0]["input_p95_ms"])
                self.assertTrue(result["errors"])

    def test_bad_frame_records_cannot_produce_valid_cadence(self):
        for frame_lines in (
            "FRAME 50000 0 1\nFRAME 150000 99999 2\n",
            "FRAME 50000 0 1\nFRAME 150000 -100000 2\n",
            "FRAME 150000 0 1\nFRAME 140000 10000 2\n",
            "FRAME 50000 0 1\nFRAME 150000 0 2\n",
            "FRAME 50000 0 1\nFRAME 150000 100000\n",
            "FRAME 50000 0 1\nFRAME 150000 nope 2\n",
        ):
            with self.subTest(frame_lines=frame_lines):
                result, phases, _ = self.summarize(frame_lines + f"KEY {START + 130_000} 30 1\n")
                self.assertFalse(result["lifecycle_passed"])
                self.assertIsNone(phases[0]["frame_p95_ms"])
                self.assertTrue(result["errors"])

    def test_empty_or_failed_run_never_passes(self):
        result, phases, _ = self.summarize("", injections=[], rows=[])
        self.assertFalse(result["lifecycle_passed"])
        self.assertIsNone(phases[0]["dbus_p95_ms"])
        result, _, _ = self.summarize(
            f"FRAME 50000 0 1\nFRAME 150000 100000 2\nKEY {START + 130_000} 30 1\n",
            completed=False,
        )
        self.assertFalse(result["lifecycle_passed"])
        self.assertEqual(result["performance"]["status"], "not_evaluated")


if __name__ == "__main__":
    unittest.main()
