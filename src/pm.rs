use crate::distro::Family;
use crate::config;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use which::which;

pub fn set_default(fam: Family, box_name: &str) -> Result<()> {
    let mut cfg = config::load_config();
    cfg.pm_defaults.insert(family_key(fam).into(), box_name.to_string());
    config::save_config(&cfg)
}

pub fn show_defaults() -> HashMap<String, String> {
    config::load_config().pm_defaults
}

pub fn generate_shims() -> Result<()> {
    let cfg = config::load_config();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let bindir: PathBuf = std::env::var("XDG_BIN_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(format!("{home}/.local/bin")));
    fs::create_dir_all(&bindir).ok();
    // Also create bootstrap shims for missing managers if no defaults exist yet
    generate_bootstrap_shims_into(&bindir)?;
    for (fam_key, box_name) in cfg.pm_defaults.iter() {
        match fam_key.as_str() {
            // Treat Ubuntu the same as Debian for apt-based shims
            "debian" | "ubuntu" => {
                generate_shim_with_policy(&bindir, "apt", box_name, fam_key)?;
                generate_shim_with_policy(&bindir, "apt-get", box_name, fam_key)?;
            }
            "fedora" => {
                generate_shim_with_policy(&bindir, "dnf", box_name, fam_key)?;
            }
            "opensuse" => {
                generate_shim_with_policy(&bindir, "zypper", box_name, fam_key)?;
            }
            "arch" => {
                generate_shim_with_policy(&bindir, "pacman", box_name, fam_key)?;
            }
            _ => {}
        }
    }
    // Ensure the bin dir is on PATH for common shells (fish gets an auto-conf.d drop-in)
    ensure_bindir_on_path(&bindir)?;
    Ok(())
}

pub fn write_shim(dir: &PathBuf, wrapper_name: &str, inner_cmd: &str, box_name: &str, fam_key: &str) -> Result<()> {
    let path = dir.join(wrapper_name);
    // Never use container root; prefer user + sudo/doas. This forwards password prompts to the host terminal.
    let content = format!("#!/usr/bin/env sh\nset -e\nbox=\"{}\"\nfam=\"{}\"\n# Fast-path readonly queries without sudo to avoid prompts\ncase \"$1\" in\n  --version|-v|--help|-h)\n    exec distrobox enter -n \"$box\" -- {} \"$@\";;\n  *) ;;\nesac\n# Pre-transaction snapshot\npkgbridge pm snapshot --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nstatus=0\n# Run inside container as user; prefer sudo, then doas, else non-root (may fail)\nif distrobox enter -n \"$box\" -- sh -lc 'command -v sudo >/dev/null' >/dev/null 2>&1; then\n  # If passwordless sudo works, great; otherwise allow interactive prompt\n  if distrobox enter -n \"$box\" -- sudo -n true >/dev/null 2>&1; then\n    distrobox enter -n \"$box\" -- sudo {} \"$@\" || status=$?\n  else\n    distrobox enter -n \"$box\" -- sudo {} \"$@\" || status=$?\n  fi\nelif distrobox enter -n \"$box\" -- sh -lc 'command -v doas >/dev/null' >/dev/null 2>&1; then\n  distrobox enter -n \"$box\" -- doas {} \"$@\" || status=$?\nelse\n  distrobox enter -n \"$box\" -- {} \"$@\" || status=$?\nfi\n# Post-transaction export\npkgbridge pm post-transaction --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nexit $status\n", box_name, fam_key, inner_cmd, inner_cmd, inner_cmd, inner_cmd, inner_cmd);
    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    let mut perms = fs::metadata(&path)?.permissions();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

pub fn family_key(f: Family) -> &'static str {
    match f { Family::Debian => "debian", Family::Fedora => "fedora", Family::OpenSuse => "opensuse", Family::Arch => "arch" }
}

fn generate_shim_with_policy(bindir: &PathBuf, name: &str, box_name: &str, fam_key: &str) -> Result<()> {
    // If the host already has this package manager (and it's not our own shim in bindir),
    // avoid overshadowing it. Instead, create a suffixed wrapper like "apt-<box>".
    let host_has = host_has_cmd_outside_bindir(name, bindir);
    let target = bindir.join(name);
    if host_has {
        let alt = format!("{}-{}", name, sanitize(box_name));
        let alt_path = bindir.join(&alt);
        if alt_path.exists() {
            // Nothing to do; do not overwrite existing
            return Ok(());
        }
        write_shim(bindir, &alt, name, box_name, fam_key)?;
        println!("Host has '{}'; created '{}' instead", name, alt);
        return Ok(());
    }
    // Host doesn't have this manager on PATH (or only our own bindir entry): prefer unsuffixed name.
    if target.exists() {
        // Do not overwrite existing file; also provide suffixed variant for clarity
        let alt = format!("{}-{}", name, sanitize(box_name));
        let alt_path = bindir.join(&alt);
        if !alt_path.exists() {
            write_shim(bindir, &alt, name, box_name, fam_key)?;
            println!("'{}' exists; created '{}' as well", name, alt);
        }
        return Ok(());
    }
    write_shim(bindir, name, name, box_name, fam_key)
}

fn host_has_cmd_outside_bindir(cmd: &str, bindir: &PathBuf) -> bool {
    match which(cmd) {
        Ok(path) => {
            // If resolved path is inside our bindir, treat as not a host tool
            let rp = path.canonicalize().unwrap_or(path);
            let rb = bindir.canonicalize().unwrap_or(bindir.clone());
            !rp.starts_with(&rb)
        }
        Err(_) => false,
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn ensure_bindir_on_path(bindir: &PathBuf) -> Result<()> {
    let bindir_str = bindir.to_string_lossy().to_string();
    // If already present, nothing to do
    if std::env::var_os("PATH")
        .and_then(|v| v.into_string().ok())
        .map(|p| p.split(':').any(|s| s == bindir_str))
        .unwrap_or(false)
    {
        return Ok(());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let shell = std::env::var("SHELL").unwrap_or_default();
    // Prefer a safe, shell-specific approach
    if shell.ends_with("fish") || std::env::var("FISH_VERSION").is_ok() {
        // Drop a conf.d snippet that adds the path for all sessions
        let confd = PathBuf::from(format!("{home}/.config/fish/conf.d"));
        fs::create_dir_all(&confd).ok();
        let snip = confd.join("pkgbridge.fish");
        let content = format!("# Added by pkgbridge: ensure user bin dir on PATH\nfish_add_path -g {}\n", bindir_str);
        // Write only if not already present or if content differs
        let write = match fs::read_to_string(&snip) { Ok(s) => s.trim() != content.trim(), Err(_) => true };
        if write { fs::write(&snip, content).with_context(|| format!("writing {}", snip.display()))?; }
        println!("Added fish PATH snippet: {} (restart fish or open a new terminal)", snip.display());
        return Ok(());
    }
    // For bash/zsh: append an idempotent block to ~/.profile
    let profile = PathBuf::from(format!("{home}/.profile"));
    let marker = "# pkgbridge: add user bin to PATH";
    let block = format!(
        "{}\nif [ -d \"{}\" ] && ! printf %s \"\n:$PATH:\n\" | grep -q \"\n:{}:\n\"; then\n  export PATH=\"{}:$PATH\"\nfi\n",
        marker, bindir_str, bindir_str, bindir_str
    );
    let mut need_write = true;
    if let Ok(existing) = fs::read_to_string(&profile) {
        if existing.contains(&bindir_str) || existing.contains(marker) { need_write = false; }
    }
    if need_write {
        let _ = fs::OpenOptions::new().create(true).append(true).open(&profile)
            .and_then(|mut f| {
                use std::io::Write as _;
                writeln!(f, "\n{}", block)
            });
        println!("Ensured PATH in {} (restart your shell)", profile.display());
    }
    Ok(())
}

fn default_box_for_family_key(fam_key: &str) -> (&'static str, &'static str) {
    match fam_key {
        "debian" | "ubuntu" => ("debian-stable", "docker.io/library/debian:stable"),
        "fedora" => ("fedora-latest", "registry.fedoraproject.org/fedora:latest"),
        "opensuse" => ("opensuse-tumbleweed", "registry.opensuse.org/opensuse/tumbleweed:latest"),
        "arch" => ("arch", "docker.io/library/archlinux:latest"),
        _ => ("distro", ""),
    }
}

fn write_bootstrap_shim(dir: &PathBuf, wrapper_name: &str, fam_key: &str, mgr: &str) -> Result<()> {
    let path = dir.join(wrapper_name);
    let (def_name, def_img) = default_box_for_family_key(fam_key);
    let content = format!("#!/usr/bin/env sh\nset -e\nfam=\"{}\"\nmgr=\"{}\"\ndef_name=\"{}\"\ndef_img=\"{}\"\n# If a default exists, use it; else offer to create one\nbox=$(pkgbridge pm show-defaults 2>/dev/null | awk -v f=\"$fam\" '$1==f && $2==\"=>\" {{print $3; exit}}')\nif [ -z \"$box\" ]; then\n  if [ -t 0 ]; then\n    echo \"pkgbridge: '$mgr' not found on host.\"\n    printf \"Create a %s box '%s' from '%s' and run '%s' from it? [Y/n] \" \"$fam\" \"$def_name\" \"$def_img\" \"$mgr\"\n    read ans || true\n    case \"$ans\" in \n      ''|y|Y|yes|YES)\n        distrobox create --name \"$def_name\" --image \"$def_img\" -Y --yes\n        pkgbridge pm set-default \"$fam\" \"$def_name\"\n        box=\"$def_name\"\n        ;;\n      *) echo \"Aborting.\"; exit 1;;\n    esac\n  else\n    echo \"pkgbridge: no default $fam container; run: pkgbridge pm set-default $fam $def_name; then pkgbridge pm generate-shims\"\n    exit 1\n  fi\nfi\n# Hand off to proper shim behavior (snapshot + sudo inside box)\npkgbridge pm snapshot --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nstatus=0\nif distrobox enter -n \"$box\" -- sh -lc 'command -v sudo >/dev/null' >/dev/null 2>&1; then\n  if distrobox enter -n \"$box\" -- sudo -n true >/dev/null 2>&1; then\n    distrobox enter -n \"$box\" -- sudo {} \"$@\" || status=$?\n  else\n    distrobox enter -n \"$box\" -- sudo {} \"$@\" || status=$?\n  fi\nelif distrobox enter -n \"$box\" -- sh -lc 'command -v doas >/dev/null' >/dev/null 2>&1; then\n  distrobox enter -n \"$box\" -- doas {} \"$@\" || status=$?\nelse\n  distrobox enter -n \"$box\" -- {} \"$@\" || status=$?\nfi\npkgbridge pm post-transaction --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nexit $status\n", fam_key, mgr, def_name, def_img, mgr, mgr, mgr, mgr);
    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    let mut perms = fs::metadata(&path)?.permissions();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

fn generate_bootstrap_shims_into(bindir: &PathBuf) -> Result<()> {
    // Create bootstrap shims only if the host doesn't already provide these managers
    // Debian/Ubuntu
    if !host_has_cmd_outside_bindir("apt", bindir) {
        write_bootstrap_shim(bindir, "apt", "debian", "apt").ok();
    }
    if !host_has_cmd_outside_bindir("apt-get", bindir) {
        write_bootstrap_shim(bindir, "apt-get", "debian", "apt-get").ok();
    }
    // Fedora
    if !host_has_cmd_outside_bindir("dnf", bindir) {
        write_bootstrap_shim(bindir, "dnf", "fedora", "dnf").ok();
    }
    // openSUSE
    if !host_has_cmd_outside_bindir("zypper", bindir) {
        write_bootstrap_shim(bindir, "zypper", "opensuse", "zypper").ok();
    }
    // Arch
    if !host_has_cmd_outside_bindir("pacman", bindir) {
        write_bootstrap_shim(bindir, "pacman", "arch", "pacman").ok();
    }
    Ok(())
}

pub fn generate_bootstrap_shims() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let bindir: PathBuf = std::env::var("XDG_BIN_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(format!("{home}/.local/bin")));
    fs::create_dir_all(&bindir).ok();
    generate_bootstrap_shims_into(&bindir)?;
    ensure_bindir_on_path(&bindir)?;
    Ok(())
}
