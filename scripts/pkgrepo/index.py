"""The files that are not packages and not signed metadata: index pages and ready-made client configuration."""

from __future__ import annotations

from html import escape
from pathlib import Path

PROJECTS = ("gnoblin", "bingux")
CHANNELS = ("stable", "nightly")

STYLE = (
    "body{font:16px/1.5 system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem}"
    "pre{background:#f4f4f4;padding:.75rem;overflow:auto}table{border-collapse:collapse;width:100%}"
    "td,th{border-bottom:1px solid #ddd;padding:.25rem .5rem;text-align:left}code{font-family:monospace}"
)


def _page(title: str, body: str) -> str:
    return (
        f'<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><title>{escape(title)}</title>'
        f'<meta name="viewport" content="width=device-width,initial-scale=1"><style>{STYLE}</style></head>'
        f"<body>\n{body}\n</body></html>\n"
    )


def fingerprint_text(fingerprint: str) -> str:
    return " ".join(fingerprint[i : i + 4] for i in range(0, len(fingerprint), 4))


def rpm_repo_file(base_url: str, project: str, channel: str, location: str, fingerprint: str) -> str:
    return (
        f"[{project}-{channel}]\n"
        f"name={project.capitalize()} {channel} ($basearch)\n"
        f"baseurl={base_url}/{project}/{channel}/{location}\n"
        "enabled=1\n"
        "gpgcheck=1\n"
        "repo_gpgcheck=1\n"
        f"gpgkey={base_url}/gpg.key\n"
        "type=rpm-md\n"
    )


def deb_sources_file(base_url: str, project: str, channel: str, suite: str) -> str:
    return (
        "Types: deb\n"
        f"URIs: {base_url}/{project}/{channel}/deb\n"
        f"Suites: {suite}\n"
        "Components: main\n"
        f"Signed-By: /etc/apt/keyrings/{project}.asc\n"
    )


def pacman_snippet(base_url: str, project: str, channel: str, name: str) -> str:
    return f"[{name}]\nSigLevel = Required DatabaseRequired\nServer = {base_url}/{project}/{channel}/arch/$arch\n"


def apk_repositories_line(base_url: str, project: str, channel: str, branch: str) -> str:
    """The line for /etc/apk/repositories. apk adds /<arch>/APKINDEX.tar.gz to it."""
    return f"{base_url}/{project}/{channel}/alpine/{branch}\n"


def write_channel(settings, root: Path, manifest: dict, arch_dirs: dict) -> None:
    from .tree import pacman_repo_name

    base, project, channel = settings.base_url, settings.project, settings.channel
    fingerprint = manifest["key"]["fingerprint"]
    families = {(p["family"], p["release"]) for p in manifest["packages"]}
    for family, release in sorted(families):
        if family == "fedora":
            (root / "fedora" / f"{project}-{channel}.repo").write_text(
                rpm_repo_file(base, project, channel, "fedora/$releasever/$basearch", fingerprint)
            )
        elif family == "opensuse":
            (root / "opensuse" / release / f"{project}-{channel}.repo").write_text(
                rpm_repo_file(base, project, channel, f"opensuse/{release}/$basearch", fingerprint)
            )
        elif family == "deb":
            (root / "deb" / f"{project}-{channel}-{release}.sources").write_text(
                deb_sources_file(base, project, channel, release)
            )
        elif family == "arch":
            name = pacman_repo_name(project, channel)
            (root / "arch" / f"{name}.conf").write_text(pacman_snippet(base, project, channel, name))
        elif family == "alpine":
            (root / "alpine" / release / f"{project}-{channel}.list").write_text(
                apk_repositories_line(base, project, channel, release)
            )

    rows = "".join(
        "<tr><td>{family} {release}</td><td>{name}</td><td>{version}</td><td>{arch}</td>"
        '<td><a href="{path}">{file}</a></td><td><code>{sha}</code></td></tr>\n'.format(
            family=escape(p["family"]),
            release=escape(p["release"]),
            name=escape(p["name"]),
            version=escape(p["version"]),
            arch=escape(p["arch"]),
            path=escape(p["path"]),
            file=escape(p["filename"]),
            sha=escape(p["sha256"][:16]),
        )
        for p in manifest["packages"]
    )
    source = manifest["source"]["commit"]
    apk_key = manifest.get("apk_key")
    apk_note = (
        f'<p>Alpine signing key: <a href="../../{escape(apk_key["name"])}">{escape(apk_key["name"])}</a>. '
        f"Install it as <code>/etc/apk/keys/{escape(apk_key['name'])}</code>.</p>\n"
        if apk_key
        else ""
    )
    body = (
        f"<h1>{escape(project.capitalize())} {escape(channel)}</h1>\n"
        f"<p>Signing key fingerprint: <code>{escape(fingerprint_text(fingerprint))}</code>"
        f' (<a href="../../gpg.key">gpg.key</a>). Generated {escape(manifest["generated"])}'
        + (f" from commit <code>{escape(source[:12])}</code>" if source else "")
        + '. <a href="manifest.json">manifest.json</a></p>\n'
        + apk_note
        + "<p>Add this repository with the instructions in the "
        f'<a href="https://github.com/kierandrewett/{escape(project)}">{escape(project.capitalize())} documentation</a>.</p>\n'
        "<table><tr><th>For</th><th>Package</th><th>Version</th><th>Architecture</th><th>File</th><th>SHA-256</th></tr>\n"
        f"{rows}</table>"
    )
    (root / "index.html").write_text(_page(f"{project.capitalize()} {channel} packages", body))


def write_root(settings, fingerprint: str, apk_key: str | None = None) -> None:
    items = "".join(
        f'<li><a href="{project}/{channel}/">{project.capitalize()} {channel}</a></li>\n'
        for project in PROJECTS
        for channel in CHANNELS
    )
    body = (
        "<h1>Gnoblin and Bingux package repositories</h1>\n"
        f'<p>Signing key: <a href="gpg.key">gpg.key</a>, fingerprint <code>{escape(fingerprint_text(fingerprint))}</code>.'
        + (f' Alpine signing key: <a href="{escape(apk_key)}">{escape(apk_key)}</a>.' if apk_key else "")
        + "</p>\n"
        "<p>Add one channel of a project to a machine. <em>stable</em> carries released versions, "
        "<em>nightly</em> the newest build of the main branch.</p>\n"
        f"<ul>\n{items}</ul>"
    )
    (settings.output / "index.html").write_text(_page("Gnoblin and Bingux package repositories", body))
