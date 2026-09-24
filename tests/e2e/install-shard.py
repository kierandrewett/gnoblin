#!/usr/bin/env python3
"""Install one app shard and retain per-app diagnostics for later triage."""

from __future__ import annotations

from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys


def run(command: list[str], timeout: int, log: Path) -> tuple[int, str]:
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        output = result.stdout + result.stderr
        log.write_text(output)
        return result.returncode, output[-8000:]
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        output = stdout + stderr + f"\nTimed out after {timeout}s\n"
        log.write_text(output)
        return 124, output[-8000:]


def main() -> int:
    shard_path = Path(sys.argv[1])
    report_path = Path(sys.argv[2])
    log_dir = report_path.parent / "install-logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    shard = json.loads(shard_path.read_text())
    rpm_apps: dict[str, list[dict]] = {}
    flatpak_apps: list[dict] = []
    for app in shard["apps"]:
        if app["source"] == "flathub-popular":
            flatpak_apps.append(app)
        elif app["source"] == "fedora-appstream":
            rpm_apps.setdefault(app["install"], []).append(app)
        else:
            raise ValueError(f"unknown catalog source: {app['source']}")

    results: dict[str, dict] = {}
    rpm_status: dict[str, dict] = {}
    packages = sorted(rpm_apps)
    if packages:
        code, output = run(["dnf", "-y", "--setopt=install_weak_deps=False", "install", *packages],
                           2400, log_dir / "rpm-batch.log")
        if code == 0:
            rpm_status = {package: {"status": "installed"} for package in packages}
        else:
            for index, package in enumerate(packages):
                code, individual_output = run(
                    ["dnf", "-y", "--setopt=install_weak_deps=False", "install", package],
                    300, log_dir / f"rpm-{index:03d}.log",
                )
                rpm_status[package] = {
                    "status": "installed" if code == 0 else "install-failed",
                    "return_code": code,
                    "diagnostic": individual_output,
                }

    results.update({
        app["app_id"]: {"app_id": app["app_id"], "source": app["source"],
                        "status": rpm_status.get(app["install"], {}).get("status", "install-failed"),
                        "package": app["install"], "details": rpm_status.get(app["install"], {})}
        for apps in rpm_apps.values() for app in apps
    })

    for index, app in enumerate(flatpak_apps):
        code, output = run(
            ["flatpak", "install", "--system", "--noninteractive", "--assumeyes",
             "flathub", app["install"]],
            1500,
            log_dir / f"flatpak-{index:03d}.log",
        )
        results[app["app_id"]] = {
            "app_id": app["app_id"], "source": app["source"],
            "status": "installed" if code == 0 else "install-failed",
            "return_code": code, "diagnostic": output,
        }
        print(f"install {app['source']} {app['app_id']}: {results[app['app_id']]['status']}", flush=True)

    report = {
        "shard": shard["shard"],
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "installed": sum(result["status"] == "installed" for result in results.values()),
        "failed": sum(result["status"] != "installed" for result in results.values()),
        "apps": list(results.values()),
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Install report: {report_path}; {report['installed']} installed, {report['failed']} failed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"shard installation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
