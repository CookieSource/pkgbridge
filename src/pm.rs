use crate::distro::Family;
use crate::config;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    for (fam_key, box_name) in cfg.pm_defaults.iter() {
        match fam_key.as_str() {
            "debian" => {
                write_shim(&bindir, "apt", box_name, fam_key)?;
                write_shim(&bindir, "apt-get", box_name, fam_key)?;
            }
            "fedora" => {
                write_shim(&bindir, "dnf", box_name, fam_key)?;
            }
            "opensuse" => {
                write_shim(&bindir, "zypper", box_name, fam_key)?;
            }
            "arch" => {
                write_shim(&bindir, "pacman", box_name, fam_key)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn write_shim(dir: &PathBuf, name: &str, box_name: &str, fam_key: &str) -> Result<()> {
    let path = dir.join(name);
    let content = format!("#!/usr/bin/env sh\nset -e\nbox=\"{}\"\nfam=\"{}\"\n# Pre-transaction snapshot\npkgbridge pm snapshot --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\n# Run manager inside box\ndistrobox enter -n \"$box\" -- {} \"$@\"\nstatus=$?\n# Post-transaction export\npkgbridge pm post-transaction --family \"$fam\" --container \"$box\" >/dev/null 2>&1 || true\nexit $status\n", box_name, fam_key, name);
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
