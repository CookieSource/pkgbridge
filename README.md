# pkgbridge — Cross-distro package installer/exporter for Distrobox (Rust)

Install native packages (`.deb`, `.rpm`, etc.) **into the right Distrobox**, then **expose both CLI binaries and desktop apps** back on your host. Also exposes **package managers themselves** (apt, dnf, zypper, pacman) as host-visible shims that operate *inside* a chosen Distrobox. Designed as a **CLI**, but integrates with your desktop so **double-clicking a package file** “just works”.

---

## 0) Terminology (chosen for consistency)

- **Package**: a software artifact distributed as `.deb` or `.rpm` (v1 target).
- **Distribution family**: a family of Linux distros with compatible package managers, e.g. `debian/ubuntu` (APT/DPKG), `fedora` (DNF/RPM), `opensuse` (zypper/RPM), `arch` (pacman).
- **Box**: a Distrobox container (rootless, backed by Podman or Docker).
- **Export**: creating host-visible shims and desktop entries that launch commands *inside* a box.

---

## 1) Goals

- From host UI: double-click `.deb`/`.rpm` → choose/auto-create a suitable box → install → export CLI + desktop app.
- From host shell: `pkgbridge install file.rpm` (non-interactive friendly).
- Expose package managers (apt/dnf/zypper/pacman) to the host as **shims** so users can run:
  - `apt install …` (executed inside a Debian/Ubuntu box)
  - `dnf install …` (inside a Fedora box)
  - `zypper install …` (inside an openSUSE box)
  - `pacman -S …` (inside an Arch box)
- **Auto-detect newly installed CLI apps** after any package-manager transaction (install/upgrade) and export them to the host (both bin shims and `.desktop` files where applicable).

---

## 2) High-level architecture

```
Host
├─ pkgbridge (Rust CLI)
│  ├─ Package file handler (for double-click; xdg-mime)
│  ├─ Format detection (.deb/.rpm)
│  ├─ Box discovery/creation (distro family classification)
│  ├─ Install adapters (dpkg/apt | rpm/dnf/zypper | pacman later)
│  ├─ Package-manager shims (apt/dnf/zypper/pacman → box)
│  ├─ Exporter (distrobox-export --bin/--app + custom shims)
│  ├─ Post-transaction scanner (detect new CLIs/desktop files)
│  └─ Config/state (~/.config/pkgbridge, ~/.local/state/pkgbridge)
└─ Distrobox containers (Debian/Ubuntu, Fedora, openSUSE, Arch)
```

**External dependencies**: `distrobox` + rootless container backend (Podman recommended, Docker supported), distro package managers inside boxes.

---

## 3) Feature checklist (replace `[ ]` with `[x]` as you implement)
- (full checklist omitted for brevity — see previous spec for details)

---

## 4) CLI design (`pkgbridge --help`)

```text
pkgbridge 1.0.0
Install native packages into Distrobox containers and export CLIs/desktop apps to the host.
Also exposes package managers (apt/dnf/zypper/pacman) as host shims.

USAGE:
  pkgbridge <COMMAND> [OPTIONS]

COMMANDS:
  open <file>                         Entry for MIME (double-click); auto-select/create a box and install.
  install <file>                      Install a .deb or .rpm into a suitable box and export.
  uninstall <pkg> [--container NAME]  Uninstall a package from a box and clean exports.
  export <pkg>                        Re-export binaries/desktop entries for an installed package.
  list                                List boxes, exports, or both.
  pm                                  Manage package-manager shims and defaults (see 'pm --help').
  doctor                              Check environment (distrobox/podman/docker, XDG dirs, permissions).
  help                                Show this help.
```

---

## 5) Man page

(see `docs/pkgbridge.1`)

---

## 6) Build from source (Rust)

```bash
git clone https://example.com/pkgbridge.git
cd pkgbridge
cargo build --release
install -Dm755 target/release/pkgbridge ~/.local/bin/pkgbridge
```
