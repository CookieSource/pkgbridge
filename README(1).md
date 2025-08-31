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

### 3.1 Core
- [ ] Detect package format: `.deb` vs `.rpm`.
- [ ] Discover existing Distrobox containers.
- [ ] Classify each box into a **distribution family** via `/etc/os-release`.
- [ ] Choose matching box for package (DEB→Debian/Ubuntu; RPM→Fedora/openSUSE).
- [ ] Auto-create a recommended box if none exist for the required family.
- [ ] Install package inside chosen box.
- [ ] Export **CLI binaries** to host (`~/.local/bin` shims).
- [ ] Export **desktop apps** to host (`~/.local/share/applications/*.desktop`).
- [ ] MIME integration so double-clicking `.deb`/`.rpm` invokes `pkgbridge open %f`.

### 3.2 Package-manager shims (run inside boxes)
- [ ] Generate host shims for **apt**, **dnf**, **zypper**, **pacman**.
- [ ] Allow default box per family (e.g., `apt` → `debian-stable`).
- [ ] Support `--container` override to run a manager in a specific box.
- [ ] Provide `pkgbridge pm set-default <family> <box>` to select defaults.

### 3.3 Post-transaction auto-export
- [ ] Detect new packages after installs/upgrades.
- [ ] Extract binaries + desktop files.
- [ ] Export automatically.

### 3.4 Export strategy & collisions
- [ ] Shims in `~/.local/bin`.
- [ ] `.desktop` entries under `~/.local/share/applications`.
- [ ] Handle name collisions.

### 3.5 Desktop integration
- [ ] MIME handlers & desktop file.
- [ ] Notifications.
- [ ] Prompts for box selection.

### 3.6 CLI & UX
- [ ] Stable CLI (`pkgbridge --help`).
- [ ] `doctor` diagnostics.
- [ ] Logs.

### 3.7 Config & policy
- [ ] TOML config under `~/.config/pkgbridge/`.
- [ ] Policy toggles.

### 3.8 Uninstall & maintenance
- [ ] `uninstall` command.
- [ ] `export` re-run.
- [ ] `list` command.
- [ ] `pm` management.

### 3.9 Security & constraints
- [ ] Rootless containers only.
- [ ] Signature verification policy.
- [ ] Arch mismatch detection.

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

GLOBAL OPTIONS:
  -c, --container <NAME>              Force a specific box by name.
  -f, --family <debian|ubuntu|fedora|opensuse|arch>
      --create                        Auto-create a box if none fits.
      --create-image <REF>            Override base image when auto-creating.
      --no-export                     Skip export.
      --bin <name>[,name...]          Export only these binaries.
      --app <name>[,name...]          Export only these desktop apps.
  -y, --yes                           Assume “yes” to prompts.
      --log-level <trace|debug|info|warn|error>
      --dry-run                       Print actions without executing them.
```

---

## 5) Man page (`pkgbridge(1)`)

(see docs/pkgbridge.1 in full text from spec)

---

## 6) Build from source (Rust)

```bash
git clone https://example.com/pkgbridge.git
cd pkgbridge
cargo build --release
install -Dm755 target/release/pkgbridge ~/.local/bin/pkgbridge
```

---

## 7) Configuration (`~/.config/pkgbridge/config.toml`)

(see example TOML config in spec)

---

## 8) Implementation notes

- Post-transaction detection of installed apps.
- Export details and shim templates.
- Box selection logic and prompting.
- Error cases.

---

## 9) File associations (host)

(see pkgbridge.desktop snippet in spec)

---

## 10) Security posture

- Rootless containers only.
- Signature enforcement optional.
- Clean logging.

---

## 11) Minimal Rust layout

```
src/
  main.rs
  cli.rs
  mime.rs
  pkgdetect.rs
  distro.rs
  installers/
    deb.rs
    rpm.rs
    pacman.rs
  export.rs
  pm.rs
  config.rs
  scanner.rs
```

---

## 12) Testing checklist

- Unit tests (format detection, config parsing, etc.).
- Integration (Debian & Fedora boxes).
- Double-click simulation.
- Package-manager shims auto-export new apps.
- Collision handling.
- DEs: GNOME, KDE, Xfce.
- Backends: Podman, Docker.
