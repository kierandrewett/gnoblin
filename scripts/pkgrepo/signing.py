"""GnuPG signing for the repository: package signatures, metadata signatures and the public key.

The private key never touches the repository tree. A CI job imports it into a throw-away GNUPGHOME from the secret
REPO_GPG_PRIVATE_KEY, and the passphrase comes from REPO_GPG_PASSPHRASE (both names are options). Nothing here
generates a key: creating the real key is a one-time offline step described in design/package-repositories.md.

Alpine's apk does not use GnuPG: it checks an RSA signature of APKINDEX.tar.gz against a public key in /etc/apk/keys.
ApkSigner holds that second key, from REPO_APK_PRIVATE_KEY (an unencrypted PEM file), and uses the openssl program.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

from .runner import CommandError, run

PRIVATE_KEY_ENV = "REPO_GPG_PRIVATE_KEY"
PASSPHRASE_ENV = "REPO_GPG_PASSPHRASE"
APK_PRIVATE_KEY_ENV = "REPO_APK_PRIVATE_KEY"


def gpg_home(path: Path | None = None) -> Path:
    """A private GNUPGHOME. gpg-agent sockets live inside it, so the path has to stay short."""
    home = path if path is not None else Path(tempfile.mkdtemp(prefix="pkgrepo-gpg-", dir=tempfile.gettempdir()))
    home.mkdir(mode=0o700, parents=True, exist_ok=True)
    home.chmod(0o700)
    return home


def import_private_key(home: Path, armored: str) -> str:
    """Import an ASCII-armoured private key and return its primary fingerprint."""
    if "BEGIN PGP PRIVATE KEY BLOCK" not in armored:
        raise CommandError("the private key is not an ASCII-armoured PGP private key block")
    run(["gpg", "--homedir", home, "--batch", "--import"], stdin=armored, capture=True, quiet=True)
    return only_secret_fingerprint(home)


def secret_fingerprints(home: Path) -> list[str]:
    out = run(["gpg", "--homedir", home, "--batch", "--with-colons", "--list-secret-keys"], capture=True, quiet=True)
    fingerprints: list[str] = []
    take_next = False
    for line in out.splitlines():
        fields = line.split(":")
        if fields[0] == "sec":
            take_next = True
        elif fields[0] == "fpr" and take_next:
            fingerprints.append(fields[9])
            take_next = False
    return fingerprints


def only_secret_fingerprint(home: Path) -> str:
    fingerprints = secret_fingerprints(home)
    if len(fingerprints) != 1:
        raise CommandError(f"expected exactly one secret key in {home}, found {len(fingerprints)}; pass --key-id")
    return fingerprints[0]


class Signer:
    def __init__(self, home: Path, key_id: str | None = None, passphrase: str | None = None) -> None:
        self.home = home
        self.fingerprint = self._resolve(key_id)
        self._secret_dir = Path(tempfile.mkdtemp(prefix="pkgrepo-pass-"))
        self._secret_dir.chmod(0o700)
        self.passphrase_file = self._secret_dir / "passphrase"
        # An empty file serves a key without a passphrase. The file is never created inside the repository tree.
        self.passphrase_file.write_text(passphrase or "")
        self.passphrase_file.chmod(stat.S_IRUSR | stat.S_IWUSR)

    @classmethod
    def from_environment(cls, home: Path, key_id: str | None, passphrase_env: str = PASSPHRASE_ENV) -> "Signer":
        return cls(home, key_id, os.environ.get(passphrase_env))

    def _resolve(self, key_id: str | None) -> str:
        if key_id is None:
            return only_secret_fingerprint(self.home)
        out = run(
            ["gpg", "--homedir", self.home, "--batch", "--with-colons", "--fingerprint", "--list-secret-keys", key_id],
            capture=True,
            quiet=True,
        )
        for line in out.splitlines():
            fields = line.split(":")
            if fields[0] == "fpr":
                return fields[9]
        raise CommandError(f"no secret key {key_id} in {self.home}")

    def close(self) -> None:
        shutil.rmtree(self._secret_dir, ignore_errors=True)

    def __enter__(self) -> "Signer":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def _gpg(self, *arguments: object, stdin: str | None = None) -> str:
        return run(
            [
                "gpg",
                "--homedir",
                self.home,
                "--batch",
                "--yes",
                "--no-tty",
                "--pinentry-mode",
                "loopback",
                "--passphrase-file",
                self.passphrase_file,
                "--local-user",
                self.fingerprint,
                *arguments,
            ],
            stdin=stdin,
            capture=True,
            quiet=True,
        )

    def detach(self, path: Path, output: Path, *, armor: bool) -> None:
        """Write a detached signature of path to output: armoured (.asc, .gpg of APT) or binary (.sig of pacman)."""
        self._gpg("--armor" if armor else "--no-armor", "--detach-sign", "--output", output, path)

    def clearsign(self, path: Path, output: Path) -> None:
        self._gpg("--armor", "--clearsign", "--output", output, path)

    def export_public(self, output: Path) -> None:
        run(
            ["gpg", "--homedir", self.home, "--batch", "--armor", "--export", "--output", output, self.fingerprint],
            capture=True,
            quiet=True,
        )

    def sign_rpm(self, path: Path) -> None:
        """Embed a signature in the RPM itself, which dnf checks when gpgcheck=1."""
        command = (
            "%{__gpg} gpg --batch --no-verbose --no-armor --no-secmem-warning --no-tty --pinentry-mode loopback "
            f"--passphrase-file {self.passphrase_file} -u {self.fingerprint} "
            "-sbo %{__signature_filename} --digest-algo sha256 %{__plaintext_filename}"
        )
        run(
            [
                "rpmsign",
                "--define",
                f"_gpg_path {self.home}",
                "--define",
                f"_gpg_name {self.fingerprint}",
                "--define",
                f"__gpg_sign_cmd {command}",
                "--addsign",
                path,
            ],
            capture=True,
            quiet=True,
        )


class ApkSigner:
    """Signs APKINDEX.tar.gz with an RSA key. `name` is the file name of the public key that apk looks for."""

    def __init__(self, pem: str, name: str) -> None:
        if "PRIVATE KEY" not in pem:
            raise CommandError("the Alpine signing key is not a PEM private key")
        if "ENCRYPTED" in pem:
            raise CommandError("the Alpine signing key must not have a passphrase")
        self.name = name
        self._directory = Path(tempfile.mkdtemp(prefix="pkgrepo-apk-"))
        self._directory.chmod(0o700)
        self._key = self._directory / "key.pem"
        self._key.write_text(pem if pem.endswith("\n") else pem + "\n")
        self._key.chmod(stat.S_IRUSR | stat.S_IWUSR)
        try:
            # `rsa` refuses a key that is not an RSA key, which apk cannot use.
            self.public_pem = self._openssl(["rsa", "-in", self._key, "-pubout"]).decode()
        except CommandError:
            self.close()
            raise CommandError("the Alpine signing key is not a valid unencrypted RSA private key") from None

    @classmethod
    def from_environment(cls, name: str, variable: str = APK_PRIVATE_KEY_ENV) -> "ApkSigner":
        pem = os.environ.get(variable)
        if not pem:
            raise CommandError(f"the environment variable {variable} is empty or not set")
        return cls(pem, name)

    @classmethod
    def from_file(cls, path: Path, name: str) -> "ApkSigner":
        return cls(path.read_text(), name)

    def close(self) -> None:
        shutil.rmtree(self._directory, ignore_errors=True)

    def __enter__(self) -> "ApkSigner":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def _openssl(self, arguments: list[object], data: bytes | None = None) -> bytes:
        try:
            result = subprocess.run(["openssl", *map(str, arguments)], input=data, capture_output=True, check=False)
        except FileNotFoundError as error:
            raise CommandError(f"openssl is not installed ({error})") from error
        if result.returncode != 0:
            raise CommandError(f"openssl {arguments[0]} failed: {result.stderr.decode().strip()}")
        return result.stdout

    def sign(self, data: bytes) -> bytes:
        return self._openssl(["dgst", "-sha256", "-sign", self._key], data)

    def write_public(self, output: Path) -> None:
        output.write_text(self.public_pem)
