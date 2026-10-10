# Gnoblin Ananicy rules

Ananicy-cpp's default rules rank `gdm` and its children as background work (nice
16, idle I/O, `SCHED_IDLE`). A Gnoblin session started by gdm inherits that
class and can freeze or drop frames while other programs use the CPU. These
files give the `gnoblin`, `quickshell` and `bingux-frame` processes normal
scheduling with nice -5.

| File               | Contents                                  |
| ------------------ | ----------------------------------------- |
| `99-gnoblin.types` | The `Gnoblin_Compositor` scheduling class |
| `99-gnoblin.rules` | Process names that use that class         |

A drop-in file cannot replace Ananicy-cpp's own rules for `gdm`, so the rules
name the Gnoblin processes. They have no effect without Ananicy-cpp.

## Packages

Each `gnoblin-ananicy-v*` release on GitHub carries these assets:

- `gnoblin-ananicy-rules-VERSION-1.noarch.rpm` for Fedora and other RPM systems
- `gnoblin-ananicy-rules-VERSION-1-any.pkg.tar.zst` for Arch Linux
- `gnoblin-ananicy-rules-VERSION.tar.gz`, the two files for any other distribution

```sh
sudo dnf install ./gnoblin-ananicy-rules-VERSION-1.noarch.rpm
sudo pacman -U gnoblin-ananicy-rules-VERSION-1-any.pkg.tar.zst
```

Each install restarts `ananicy-cpp`. Log out and back in, because a running
process keeps the scheduling class it inherited. Check the result with
`chrt -p "$(pgrep -ox gnoblin)"`: the policy should be `SCHED_OTHER`.

## Build

```sh
rpmbuild -ba gnoblin-ananicy-rules.spec   # after making the source tarball
makepkg -f                               # in this directory
```

The release workflow is `.github/workflows/ananicy-release.yml`. Update the
version in `gnoblin-ananicy-rules.spec` and `PKGBUILD` together; the workflow
checks that they match the tag.
