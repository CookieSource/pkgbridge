# pkgbridge Documentation

This guide covers commands, options, first‑run behavior, collision handling, paths, and examples.

## Commands

- open <file>
  - Entry for MIME (double‑click). Auto‑selects/creates a box and installs the package.
- install <file>
  - Install a `.deb`/`.rpm` into a suitable box and export CLIs/desktop apps.
- export --container <box> <pkg>
  - Re‑export binaries and desktop entries for an installed package inside `<box>`.
- uninstall --container <box> <pkg>
  - Remove exports, then uninstall `<pkg>` from `<box>`.
- list boxes
  - List discovered Distrobox containers, including distribution family.
- doctor
  - Environment diagnostics (container runtime, distrobox, distrobox‑export, XDG dirs, PATH).
- pm
  - set-default <family> <box>: set the default box for a distro family.
  - generate-shims: generate host shims (apt/dnf/zypper/pacman) in `~/.local/bin`.
  - show-defaults: show configured family → box mapping.
  - snapshot (internal): take a pre‑transaction snapshot; used by generated shims.
  - post-transaction (internal): detect changes and auto‑export; used by generated shims.
- desktop
  - install: install `pkgbridge.desktop` under `~/.local/share/applications` and register MIME.
  - uninstall: remove the desktop file; leaves system defaults unchanged.

## Global Options

- --container <box>: force a specific box.
- --family <debian|fedora|opensuse|arch>: prefer a distro family.
- --create: auto‑create a recommended box if none match.
- --create-image <ref>: override image for auto‑creation.
- --no-export: skip export stage (install only).
- --bin <name>[,name…]: export exactly these binaries.
- --app <base.desktop>[,base.desktop…]: export exactly these desktop apps.
- --log-level <trace|debug|info|warn|error>: set logging level (default: info).
- --dry-run: print actions without executing them.

## First‑Run Onboarding

- On the first interactive run, pkgbridge discovers existing boxes and offers to:
  - Generate package‑manager shims bound to sensible defaults (one box per family).
  - Export existing `.desktop` apps from those boxes.
- If no boxes/apps are found, nothing is shown. The prompt appears only once when interactive.

## Auto‑Export After Package Manager Transactions

- Generated shims wrap your host commands (apt/dnf/zypper/pacman) as follows:
  1. Take a pre‑transaction package snapshot inside the box.
  2. Run the package manager in the box.
  3. Diff post‑transaction packages against the snapshot.
  4. For new or upgraded packages, scan `usr/bin/*` and `usr/share/applications/*.desktop` and export them.
- Important: Auto‑export happens when you use the shims (`~/.local/bin/apt` etc.). Running managers directly inside the container will not trigger it.

## Collision Handling

- Bin shim name collision (e.g., `~/.local/bin/foo` already exists):
  - pkgbridge exports a fallback shim named `foo-<container>`.
- Desktop file collision (e.g., `~/.local/share/applications/foo.desktop` exists):
  - pkgbridge copies the container’s `.desktop`, rewrites `Exec=` to launch via `distrobox enter -n <container> -- …`, and writes `foo.<container>.desktop`.

## Paths

- Host bin directory: `${XDG_BIN_HOME:-$HOME/.local/bin}`
- Host applications directory: `${XDG_DATA_HOME:-$HOME/.local/share}/applications`
- Config: `${XDG_CONFIG_HOME:-$HOME/.config}/pkgbridge/config.toml`
- State: `${XDG_STATE_HOME:-$HOME/.local/state}/pkgbridge/state.toml`
- Snapshots: `${XDG_STATE_HOME:-$HOME/.local/state}/pkgbridge/snapshots/<container>.txt`

## Examples

Install and export a `.deb` into the first matching Debian/Ubuntu box:

```bash
pkgbridge install ~/Downloads/some-app_1.2.3_amd64.deb
```

Force Fedora family (or auto‑create if none exist):

```bash
pkgbridge install --family fedora --create ~/Downloads/some-tool-2.0-1.x86_64.rpm
```

Re‑export only certain binaries for an installed package:

```bash
pkgbridge export --container debian-stable --bin tool,helper mypackage
```

Uninstall a package and remove exports:

```bash
pkgbridge uninstall --container debian-stable mypackage
```

Generate shims and use them from the host:

```bash
pkgbridge pm set-default debian debian-stable
pkgbridge pm generate-shims
apt install htop   # runs inside debian-stable and auto‑exports
```

Enable double‑click from your desktop:

```bash
pkgbridge desktop install
```

## Known Limitations

- Requires `distrobox` and `distrobox-export` on the host; apt/dnf/zypper/pacman inside the target boxes.
- Auto‑export relies on shims; package manager runs inside the container won’t be detected.
- Signature policies and architecture mismatch checks are not enforced by default.

