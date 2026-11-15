#!/usr/bin/env python3
"""Unpack a container image from its registry into a plain directory, with no container runtime.

usage: scripts/oci-rootfs.py IMAGE DEST

IMAGE is a name as `design/package-targets.json` writes it (`fedora:45`, `quay.io/rockylinux/rockylinux:9`,
`registry.opensuse.org/opensuse/leap:16.0`). DEST becomes the image's root file system. Run it as root so the
ownership in the layers is kept: `dpkg`, `rpm` and `apk` read it back.

Why this exists: an Agent37 instance has no podman, docker or skopeo and cannot mount, but it can `chroot` as root,
so an image is only a tree of files to it. Anonymous registry access is enough for every public image the targets
name. Device nodes in a layer are skipped (mknod is refused there), and a layer's whiteouts are applied the way the
OCI image specification says. The architecture is linux/amd64 because `design/package-targets.json` says x86_64.
"""

import gzip
import json
import os
import re
import shutil
import sys
import tarfile
import urllib.error
import urllib.request

ACCEPT = ", ".join(
    [
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.docker.distribution.manifest.v2+json",
    ]
)


class PullError(Exception):
    """The image could not be fetched or unpacked; the message says where it stopped."""


def split_reference(image):
    """Registry host, repository and tag of an image name, with Docker Hub's defaults."""
    name, _, tag = image.rpartition(":")
    if not name or "/" in tag:
        name, tag = image, "latest"
    parts = name.split("/")
    if len(parts) > 1 and ("." in parts[0] or ":" in parts[0] or parts[0] == "localhost"):
        host, repository = parts[0], "/".join(parts[1:])
    else:
        host, repository = "registry-1.docker.io", name if len(parts) > 1 else "library/" + name
    return host, repository, tag


class Registry:
    """One registry with the bearer-token handshake every public registry uses."""

    def __init__(self, host):
        self.host = host
        self.token = None

    def get(self, path, accept=None, raw=False):
        url = f"https://{self.host}{path}"
        for _ in range(2):
            request = urllib.request.Request(url)
            if accept:
                request.add_header("Accept", accept)
            if self.token:
                request.add_header("Authorization", f"Bearer {self.token}")
            try:
                response = urllib.request.urlopen(request, timeout=120)
                return response if raw else response.read()
            except urllib.error.HTTPError as error:
                challenge = error.headers.get("WWW-Authenticate", "")
                if error.code != 401 or not challenge.startswith("Bearer "):
                    raise PullError(f"{url}: HTTP {error.code}") from error
                self.authenticate(challenge)
        raise PullError(f"{url}: the registry refused the token")

    def authenticate(self, challenge):
        fields = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
        realm = fields.get("realm")
        if not realm:
            raise PullError(f"no realm in the challenge {challenge!r}")
        query = "&".join(f"{key}={fields[key]}" for key in ("service", "scope") if key in fields)
        with urllib.request.urlopen(f"{realm}?{query}", timeout=60) as response:
            body = json.load(response)
        self.token = body.get("token") or body.get("access_token")
        if not self.token:
            raise PullError("the token endpoint gave no token")


def resolve_manifest(registry, repository, reference):
    """The single-platform manifest for linux/amd64, following an index when there is one."""
    manifest = json.loads(registry.get(f"/v2/{repository}/manifests/{reference}", ACCEPT))
    while "manifests" in manifest:
        chosen = next(
            (
                entry
                for entry in manifest["manifests"]
                if entry.get("platform", {}).get("os") == "linux"
                and entry.get("platform", {}).get("architecture") == "amd64"
            ),
            None,
        )
        if chosen is None:
            raise PullError("the image has no linux/amd64 manifest")
        manifest = json.loads(registry.get(f"/v2/{repository}/manifests/{chosen['digest']}", ACCEPT))
    return manifest


def remove_path(path):
    if os.path.islink(path) or os.path.isfile(path):
        os.unlink(path)
    elif os.path.isdir(path):
        shutil.rmtree(path)


def extract(stream, mode):
    """Extracts one layer over `/` with whiteouts applied. It runs after `chroot`, so a layer's absolute symlinks
    resolve inside the image and never in the host."""
    fully_trusted = getattr(tarfile, "fully_trusted_filter", None)
    with tarfile.open(fileobj=stream, mode=mode) as layer:
        for member in layer:
            name = os.path.normpath(member.name).lstrip("/")
            if name.startswith(".."):
                raise PullError(f"layer member escapes the root: {member.name}")
            directory, base = os.path.split(name)
            target_dir = os.path.join("/", directory)
            if base == ".wh..wh..opq":
                if os.path.isdir(target_dir):
                    for entry in os.listdir(target_dir):
                        remove_path(os.path.join(target_dir, entry))
                continue
            if base.startswith(".wh."):
                remove_path(os.path.join(target_dir, base[4:]))
                continue
            if member.isdev() or member.isfifo():
                continue
            path = os.path.join("/", name)
            if os.path.lexists(path) and not (member.isdir() and os.path.isdir(path) and not os.path.islink(path)):
                remove_path(path)
            if fully_trusted is not None:
                layer.extract(member, "/", set_attrs=True, filter=fully_trusted)
            else:
                layer.extract(member, "/", set_attrs=True)


def apply_layer(path, dest, gzipped):
    """Applies the layer file `path` to `dest` in a child that has changed its root there."""
    pid = os.fork()
    if pid == 0:
        code = 0
        try:
            # Opened first: after the chroot the host path no longer exists.
            with open(path, "rb") as handle:
                os.chroot(dest)
                os.chdir("/")
                if gzipped:
                    extract(gzip.GzipFile(fileobj=handle), "r|")
                else:
                    extract(handle, "r:")
        except (PullError, OSError, tarfile.TarError) as error:
            print(f"oci-rootfs: layer {path}: {error}", file=sys.stderr)
            code = 1
        finally:
            os._exit(code)
    _, status = os.waitpid(pid, 0)
    if status != 0:
        raise PullError(f"unpacking {path} failed")


def pull(image, dest):
    host, repository, reference = split_reference(image)
    registry = Registry(host)
    manifest = resolve_manifest(registry, repository, reference)
    os.makedirs(dest, exist_ok=True)
    scratch = os.path.join(os.path.dirname(os.path.abspath(dest)), ".oci-layers-" + os.path.basename(dest.rstrip("/")))
    os.makedirs(scratch, exist_ok=True)
    try:
        for index, layer in enumerate(manifest["layers"], 1):
            if "zstd" in layer.get("mediaType", ""):
                raise PullError(f"layer {index} is zstd-compressed and this host has no zstd reader")
            print(
                f"oci-rootfs: {image} layer {index}/{len(manifest['layers'])} {layer['digest'][:19]} {layer['size']} bytes",
                flush=True,
            )
            blob = os.path.join(scratch, f"{index}.tar")
            with (
                registry.get(f"/v2/{repository}/blobs/{layer['digest']}", raw=True) as response,
                open(blob, "wb") as out,
            ):
                shutil.copyfileobj(response, out)
            with open(blob, "rb") as head:
                gzipped = head.read(2) == b"\x1f\x8b"
            apply_layer(blob, os.path.abspath(dest), gzipped)
            os.unlink(blob)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        pull(argv[1], argv[2])
    except (PullError, OSError, tarfile.TarError, urllib.error.URLError) as error:
        print(f"oci-rootfs: {argv[1]}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
