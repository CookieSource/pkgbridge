use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn desktop_dir() -> PathBuf {
    std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(format!("{home}/.local/share"))
    }).join("applications")
}

pub fn desktop_file_path() -> PathBuf {
    desktop_dir().join("pkgbridge.desktop")
}

pub fn install(dry_run: bool) -> Result<()> {
    let dir = desktop_dir();
    let path = desktop_file_path();
    let content = desktop_file_content();
    if dry_run {
        println!("--dry-run: would write {}", path.display());
    } else {
        fs::create_dir_all(&dir).ok();
        fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    }
    // Register MIME associations
    let mimes = vec![
        "application/vnd.debian.binary-package",
        "application/x-deb",
        "application/x-rpm",
        "application/x-redhat-package-manager",
    ];
    let _ = try_run("update-desktop-database", &[dir.to_string_lossy().as_ref()]);
    for mt in &mimes {
        let _ = try_run("xdg-mime", &["default", "pkgbridge.desktop", mt]);
    }
    // Ensure defaults are persisted in mimeapps.list
    ensure_mimeapps_defaults(&mimes)?;
    // Provide local MIME globs in case system DB is missing entries
    install_mime_xml()?;
    // Install app icon into XDG icon theme (hicolor)
    install_icon(dry_run)?;
    // Update icon cache if available
    let _ = try_run("gtk-update-icon-cache", &[xdg_data_home().join("icons/hicolor").to_string_lossy().as_ref(), "-q"]);    
    let _ = try_run("update-mime-database", &[xdg_data_home().join("mime").to_string_lossy().as_ref()]);
    Ok(())
}

pub fn uninstall(_dry_run: bool) -> Result<()> {
    let path = desktop_file_path();
    let _ = std::fs::remove_file(&path);
    // Remove installed icon
    uninstall_icon().ok();
    let _ = try_run("gtk-update-icon-cache", &[xdg_data_home().join("icons/hicolor").to_string_lossy().as_ref(), "-q"]);
    let _ = try_run("update-desktop-database", &[desktop_dir().to_string_lossy().as_ref()]);
    // Remove mimeapps defaults that point to pkgbridge.desktop
    remove_mimeapps_defaults()?;
    Ok(())
}

pub fn desktop_file_content() -> String {
    // Minimal .desktop to handle opening .deb/.rpm
    let exec = "pkgbridge open %f";
    let mut s = String::new();
    s.push_str("[Desktop Entry]\n");
    s.push_str("Type=Application\n");
    s.push_str("Name=Pkgbridge Package Installer\n");
    s.push_str("Comment=Install native packages into Distrobox containers\n");
    s.push_str("TryExec=pkgbridge\n");
    s.push_str("Icon=pkgbridge\n");
    s.push_str(&format!("Exec={}\n", exec));
    s.push_str("Terminal=true\n");
    s.push_str("Categories=System;Utility;\n");
    s.push_str("MimeType=application/vnd.debian.binary-package;application/x-deb;application/x-rpm;application/x-redhat-package-manager;\n");
    s.push_str("NoDisplay=false\n");
    s.push_str("X-Pkgbridge=true\n");
    s
}

fn try_run(cmd: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(cmd).args(args).status();
    match status {
        Ok(_) => Ok(()),
        Err(_) => Ok(()),
    }
}

fn ensure_mimeapps_defaults(mimes: &[&str]) -> Result<()> {
    let cfg_dir = xdg_config_home();
    fs::create_dir_all(&cfg_dir).ok();
    let path = cfg_dir.join("mimeapps.list");
    let mut data = String::new();
    if let Ok(s) = fs::read_to_string(&path) { data = s; }
    let mut lines: Vec<String> = if data.is_empty() { vec![] } else { data.lines().map(|s| s.to_string()).collect() };
    // Ensure [Default Applications] section exists
    let mut idx = lines.iter().position(|l| l.trim() == "[Default Applications]");
    if idx.is_none() { lines.push("[Default Applications]".into()); idx = Some(lines.len()-1); lines.push(String::new()); }
    // Map of mime->line index under the section
    let mut i = idx.unwrap() + 1;
    let mut end = lines.len();
    for (j, l) in lines.iter().enumerate().skip(i) { if l.starts_with('[') { end = j; break; } }
    // Build a set of existing entries
    let mut existing: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (j, l) in lines.iter().enumerate().take(end).skip(i) {
        if let Some((k, _)) = l.split_once('=') { existing.insert(k.trim().to_string(), j); }
    }
    for mt in mimes {
        let entry = format!("{}=pkgbridge.desktop;", mt);
        if let Some(&j) = existing.get(&mt.to_string()) {
            lines[j] = entry;
        } else {
            lines.insert(end, entry);
            end += 1;
        }
    }
    let mut out = fs::File::create(&path).with_context(|| format!("writing {}", path.display()))?;
    for l in &lines { writeln!(out, "{}", l).ok(); }
    Ok(())
}

fn remove_mimeapps_defaults() -> Result<()> {
    let cfg_dir = xdg_config_home();
    let path = cfg_dir.join("mimeapps.list");
    let Ok(s) = fs::read_to_string(&path) else { return Ok(()); };
    let mut lines: Vec<String> = s.lines().map(|x| x.to_string()).collect();
    let mut i = match lines.iter().position(|l| l.trim() == "[Default Applications]") { Some(v) => v + 1, None => return Ok(()) };
    let mut end = lines.len();
    for (j, l) in lines.iter().enumerate().skip(i) { if l.starts_with('[') { end = j; break; } }
    let mut kept: Vec<String> = Vec::new();
    kept.extend(lines.drain(..i));
    for l in lines.drain(..end-i) {
        if l.contains("=pkgbridge.desktop;") { continue; }
        kept.push(l);
    }
    kept.extend(lines);
    let mut out = fs::File::create(&path).with_context(|| format!("writing {}", path.display()))?;
    for l in &kept { writeln!(out, "{}", l).ok(); }
    Ok(())
}

fn install_mime_xml() -> Result<()> {
    let base = xdg_data_home().join("mime").join("packages");
    fs::create_dir_all(&base).ok();
    let path = base.join("pkgbridge.xml");
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-deb">
    <glob pattern="*.deb"/>
  </mime-type>
  <mime-type type="application/vnd.debian.binary-package">
    <glob pattern="*.deb"/>
  </mime-type>
  <mime-type type="application/x-rpm">
    <glob pattern="*.rpm"/>
  </mime-type>
</mime-info>
"#;
    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// Embed icon bytes (pkgbridge.png at repository root)
const ICON_BYTES: &[u8] = include_bytes!("../pkgbridge.png");

fn icon_target_dir() -> PathBuf {
    xdg_data_home().join("icons").join("hicolor").join("256x256").join("apps")
}

fn icon_target_path() -> PathBuf { icon_target_dir().join("pkgbridge.png") }

fn install_icon(dry_run: bool) -> Result<()> {
    let dir = icon_target_dir();
    let path = icon_target_path();
    if dry_run {
        println!("--dry-run: would install icon to {}", path.display());
        return Ok(());
    }
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(&path, ICON_BYTES).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn uninstall_icon() -> Result<()> {
    let path = icon_target_path();
    if path.exists() { let _ = std::fs::remove_file(&path); }
    Ok(())
}

fn xdg_config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(format!("{home}/.config"))
    })
}

fn xdg_data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(format!("{home}/.local/share"))
    })
}
