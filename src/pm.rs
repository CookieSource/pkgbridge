use crate::config;
use crate::distro::Family;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use which::which;

pub fn set_default(fam: Family, box_name: &str) -> Result<()> {
    let mut cfg = config::load_config();
    cfg.pm_defaults
        .insert(family_key(fam).into(), box_name.to_string());
    config::save_config(&cfg)
}

pub fn show_defaults() -> HashMap<String, String> {
    config::load_config().pm_defaults
}

pub fn generate_shims() -> Result<()> {
    let cfg = config::load_config();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let bindir: PathBuf = std::env::var("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{home}/.local/bin")));
    fs::create_dir_all(&bindir).ok();
    for (fam_key, box_name) in cfg.pm_defaults.iter() {
        match fam_key.as_str() {
            "debian" => {
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
    Ok(())
}

pub fn write_shim(dir: &PathBuf, name: &str, box_name: &str, fam_key: &str) -> Result<()> {
    let path = dir.join(name);
    // Try to run the package manager as root inside the container.
    // Fallbacks: attempt with sudo/doas inside container, then non-root (may fail for ops requiring privileges).
    let content = format!("#!/usr/bin/env sh\nset -e\nbox=\"{}\"\nfam=\"{}\"\n# Pre-transaction snapshot\npkgbridge pm snapshot --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nstatus=0\nif distrobox enter --root -n \"$box\" -- true >/dev/null 2>&1; then\n  distrobox enter --root -n \"$box\" -- {} \"$@\" || status=$?\nelse\n  if distrobox enter -n \"$box\" -- command -v sudo >/dev/null 2>&1; then\n    distrobox enter -n \"$box\" -- sudo {} \"$@\" || status=$?\n  elif distrobox enter -n \"$box\" -- command -v doas >/dev/null 2>&1; then\n    distrobox enter -n \"$box\" -- doas {} \"$@\" || status=$?\n  else\n    distrobox enter -n \"$box\" -- {} \"$@\" || status=$?\n  fi\nfi\n# Post-transaction export\npkgbridge pm post-transaction --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nexit $status\n", box_name, fam_key, name, name, name, name);
    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    let mut perms = fs::metadata(&path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

pub fn family_key(f: Family) -> &'static str {
    match f {
        Family::Debian => "debian",
        Family::Fedora => "fedora",
        Family::OpenSuse => "opensuse",
        Family::Arch => "arch",
    }
}

fn generate_shim_with_policy(
    bindir: &PathBuf,
    name: &str,
    box_name: &str,
    fam_key: &str,
) -> Result<()> {
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
        write_shim(bindir, &alt, box_name, fam_key)?;
        println!("Host has '{}'; created '{}' instead", name, alt);
        return Ok(());
    }
    // Host doesn't have this manager on PATH (or only our own bindir entry): prefer unsuffixed name.
    if target.exists() {
        // Do not overwrite existing file; also provide suffixed variant for clarity
        let alt = format!("{}-{}", name, sanitize(box_name));
        let alt_path = bindir.join(&alt);
        if !alt_path.exists() {
            write_shim(bindir, &alt, box_name, fam_key)?;
            println!("'{}' exists; created '{}' as well", name, alt);
        }
        return Ok(());
    }
    write_shim(bindir, name, box_name, fam_key)
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
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
