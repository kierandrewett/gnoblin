"""Ready-made client configuration for each package format: dnf .repo, deb822 .sources, pacman and apk snippets."""

from __future__ import annotations

from pathlib import Path


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
