<div align="center">

<img src="pkgbridge.png" alt="pkgbridge" width="128" />

# pkgbridge
# NOTE THIS IS ALPHA SOFTWARE AND NOT READY FOR USE.
Cross‑distro package installer/exporter for Distrobox. Install native packages (.deb/.rpm) into the right container, then expose both CLI binaries and desktop apps back on your host. Includes host shims for apt/dnf/zypper/pacman that operate inside a chosen Distrobox.

</div>

---

## Highlights

- Install .deb/.rpm into a suitable Distrobox (auto‑create if needed)
- Export installed CLIs to `~/.local/bin` and apps to `~/.local/share/applications`
- First‑run onboarding to generate `apt`/`dnf`/`zypper`/`pacman` host shims
- Auto‑export newly installed apps after package manager transactions
- Desktop integration: double‑click `.deb`/`.rpm` → `pkgbridge open %f`
- Smart collision handling for shims and `.desktop` files

## Requirements

- Host: `distrobox`, container runtime (`podman` recommended or `docker`), `distrobox-export`
- Desktop integration: `xdg-mime`, `update-desktop-database` (from `desktop-file-utils`)
- Notifications (optional): `notify-send`

## Install

Build from source (Rust):

```bash
git clone https://example.com/pkgbridge.git
cd pkgbridge
cargo build --release
install -Dm755 target/release/pkgbridge ~/.local/bin/pkgbridge
```

No Rust? Build with Docker:

```bash
docker run --rm -v "$(pwd)":/work -w /work rust:1.84 cargo build --release
install -Dm755 target/release/pkgbridge ~/.local/bin/pkgbridge
```

Note: Some folders (e.g., OneDrive) may be mounted `noexec`. If you see “Permission denied” when running the binary from your repo, copy it elsewhere (e.g., `/tmp/pkgbridge`) before executing.

## Quick Start

1) Create a base box (example: Debian):

```bash
distrobox create --name debian-stable --image docker.io/library/debian:stable -Y
```

2) First‑run onboarding will offer to generate shims and export existing apps. Or do it explicitly:

```bash
pkgbridge pm set-default debian debian-stable
pkgbridge pm generate-shims
```

3) Enable desktop integration (double‑click):

```bash
pkgbridge desktop install
```

4) Install a package:

```bash
pkgbridge install /path/to/file.deb     # auto‑selects a Debian/Ubuntu box
pkgbridge install /path/to/file.rpm     # auto‑selects a Fedora/openSUSE box
```

5) Use host shims for package managers (`~/.local/bin`):

```bash
apt install htop      # runs inside your default Debian/Ubuntu box and auto‑exports
dnf install htop      # runs inside your default Fedora box and auto‑exports
```

## Usage Overview

Run `pkgbridge --help` for a quick overview. A full command reference lives in `docs/README.md`.

- `open <file>` — handle double‑click; auto‑select/create a box and install
- `install <file>` — install `.deb`/`.rpm` into a box and export
- `export --container <box> <pkg>` — re‑export CLIs/apps for a package
- `uninstall --container <box> <pkg>` — remove exports and uninstall package
- `list boxes` — list discovered boxes with family classification
- `pm …` — manage defaults, generate shims (apt/dnf/zypper/pacman)
- `desktop …` — install/uninstall desktop file + MIME associations
- `doctor` — environment diagnostics

Global options: `--container`, `--family`, `--create [--create-image]`, `--bin`, `--app`, `--no-export`, `--log-level`, `--dry-run`.

## How It Works

- Format detection (magic + extension) chooses DEB vs RPM
- Boxes discovered via `distrobox list`; family via `/etc/os-release`
- Installs run inside the box (root) via `distrobox enter --root`
- Exports via `distrobox-export` with graceful fallbacks and collision handling
- Host shims for package managers snapshot → run → post‑transaction auto‑export

## Troubleshooting

- “Permission denied” on binary: copy to a non‑`noexec` location (e.g., `/tmp`) and run
- `~/.local/bin` not on PATH: add it to your shell profile; `pkgbridge doctor` will point this out
- Missing `distrobox-export`/`xdg-mime`: install packages from your distro

## Documentation

Complete docs, command reference, and examples: see `docs/README.md`.

## License

Dual‑licensed under MIT or Apache 2.0. See `LICENSE`.
