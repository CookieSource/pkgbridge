use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::distro;
use crate::distro::Family as BoxFamily;
use crate::pm;
use crate::config;
use crate::desktop;
use std::io::IsTerminal;
use crate::pkgdetect::{detect_package_format, PackageFormat};
use std::path::PathBuf as StdPathBuf;

#[derive(Parser, Debug)]
#[command(name = "pkgbridge", version, about = "Install native packages into Distrobox containers and export CLIs/desktop apps to the host.")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Increase output verbosity
    #[arg(long, global = true, default_value_t = false)]
    dry_run: bool,
    /// Force a specific box by name
    #[arg(short = 'c', long, global = true)]
    container: Option<String>,
    /// Preferred distro family for selection or creation
    #[arg(long, value_enum, global = true)]
    family: Option<FamilyArg>,
    /// Auto-create a recommended box if none exist for the required family
    #[arg(long, global = true, default_value_t = false)]
    create: bool,
    /// Override base image when auto-creating
    #[arg(long, global = true)]
    create_image: Option<String>,
    /// Skip export after install
    #[arg(long, global = true, default_value_t = false)]
    no_export: bool,
    /// Export only these binaries (comma-separated or repeated)
    #[arg(long, value_delimiter = ',', global = true)]
    bin: Vec<String>,
    /// Export only these desktop apps (.desktop basenames; comma-separated or repeated)
    #[arg(long, value_delimiter = ',', global = true)]
    app: Vec<String>,
    /// Log level
    #[arg(long, value_enum, global = true)]
    log_level: Option<LogLevel>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Entry for MIME (double-click); auto-select/create a box and install.
    Open(FileArg),
    /// Install a .deb or .rpm into a suitable box and export.
    Install(FileArg),
    /// Re-export binaries/desktop entries for an installed package.
    Export(PkgArg),
    /// Uninstall a package from a box and remove exports.
    Uninstall(PkgArg),
    /// List boxes discovered via distrobox
    List(ListArgs),
    /// Check environment (distrobox, container runtime, XDG dirs)
    Doctor,
    /// Package manager defaults & shims
    Pm { #[command(subcommand)] cmd: PmCmd },
    /// Desktop integration (MIME/desktop file)
    Desktop { #[command(subcommand)] cmd: DesktopCmd },
}

#[derive(Args, Debug, Clone)]
pub struct FileArg {
    /// Path to a .deb or .rpm file
    file: PathBuf,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// What to list (currently only 'boxes')
    #[arg(value_enum, default_value_t = ListTarget::Boxes)]
    target: ListTarget,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ListTarget { Boxes }

#[derive(Args, Debug, Clone)]
pub struct PkgArg {
    /// Package name inside the container
    pkg: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PmCmd {
    /// Set default box for a distro family
    SetDefault { #[arg(value_enum)] family: FamilyArg, box_name: String },
    /// Generate shims in ~/.local/bin for configured defaults
    GenerateShims,
    /// Show configured defaults
    ShowDefaults,
    /// Take a snapshot of installed packages before a transaction
    Snapshot,
    /// Detect changes since snapshot and export new/updated apps
    PostTransaction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DesktopCmd {
    /// Install desktop file and MIME associations
    Install,
    /// Remove desktop file and leave MIME defaults unchanged
    Uninstall,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logger(cli.log_level);
    maybe_first_run_prompt();

    match &cli.command {
        Commands::Open(arg) | Commands::Install(arg) => install_like(arg.clone(), &cli),
        Commands::Export(arg) => export_pkg(&cli, arg.clone()),
        Commands::Uninstall(arg) => uninstall_pkg(&cli, arg.clone()),
        Commands::List(args) => match args.target {
            ListTarget::Boxes => {
                let boxes = distro::discover_boxes().context("discovering boxes")?;
                if boxes.is_empty() {
                    println!("No boxes found (is 'distrobox' installed?)");
                } else {
                    println!("NAME\tFAMILY\tRUNTIME\tIMAGE");
                    for b in boxes {
                        let fam = match distro::classify_box_family(&b.name) { Ok(f) => format_family(f), Err(_) => "?".into() };
                        println!("{}\t{}\t{}\t{}", b.name, fam, b.runtime, b.image.unwrap_or_default());
                    }
                }
                Ok(())
            }
        },
        Commands::Doctor => doctor(),
        Commands::Pm { cmd } => pm_cmd(cmd.clone()),
        Commands::Desktop { cmd } => desktop_cmd(cmd.clone(), cli.dry_run),
    }
}

fn install_like(arg: FileArg, cli: &Cli) -> Result<()> {
    let path = arg.file;
    if !path.exists() {
        return Err(anyhow!("file does not exist: {}", path.display()));
    }

    let fmt = detect_package_format(&path).context("detecting package format")?;
    println!("Detected format: {}", match fmt { PackageFormat::Deb => "deb", PackageFormat::Rpm => "rpm"});
    let containers = distro::discover_boxes().unwrap_or_default();
    let selected = select_or_create(&containers, &fmt, cli)?;
    println!("Selected box: {} (family: {})", selected.name, format_family(selected.family));
    println!("Plan: install {} inside '{}'", path.display(), selected.name);
    if cli.dry_run {
        println!("--dry-run: stopping before any installation/export work.");
        return Ok(());
    }
    // If non-interactive and a password seed is provided, set it before any container entry
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !interactive {
        if let Ok(pw) = std::env::var("PKGBRIDGE_INIT_PASSWORD") {
            preseed_password_root(&selected.name, &pw).ok();
        }
    }
    // Copy the package into the container to a temp path
    let in_box_path = distro::copy_into_box(&selected.name, &path).context("copying package into container")?;
    // Verify copy size to avoid corrupted installs due to TTY/pipe issues
    if let Ok(meta) = std::fs::metadata(&path) {
        let host_sz = meta.len();
        let q = shell_escape::escape(std::borrow::Cow::from(in_box_path.clone()));
        let cmd = format!("stat -c %s {} 2>/dev/null || wc -c < {} 2>/dev/null", q, q);
        if let Ok(out) = distro::enter_capture(&selected.name, &cmd, false) {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Some(tok) = s.split_whitespace().next() {
                        if let Ok(n) = tok.trim().parse::<u64>() {
                            if n != host_sz { return Err(anyhow!("copied file size mismatch inside container: expected {} bytes, got {} (path: {})", host_sz, n, in_box_path)); }
                        }
                    }
                }
            }
        }
    }
    // Pre-scan contents to identify bins and desktop files
    let (mut bins, mut apps) = prescan_package(&selected.name, &fmt, &in_box_path)?;
    if !cli.bin.is_empty() { bins = cli.bin.clone(); }
    if !cli.app.is_empty() { apps = cli.app.clone(); }
    // Build both user and root install commands. Prefer user+sudo in interactive sessions
    // to forward password prompts; fallback to root if needed.
    let user_cmd = build_install_cmd_user(&fmt, &in_box_path);
    let root_cmd = build_install_cmd_root(&fmt, &in_box_path);
    println!("Installing inside box '{}'...", selected.name);

    // Interactive or non-interactive execution depending on TTY

    // Prefer interactive execution to forward password prompts to user
    let ok = if interactive {
        // 1) Try as normal user (sudo/doas will prompt interactively)
        log::debug!("install (user) cmd: {}", user_cmd);
        match distro::enter_status_inherit(&selected.name, &user_cmd, false) {
            Ok(true) => true,
            _ => {
                // 2) Fallback to root (no prompts)
                log::debug!("install (root fallback) cmd: {}", root_cmd);
                match distro::enter_status_inherit(&selected.name, &root_cmd, true) {
                    Ok(true) => true,
                    Ok(false) => false,
                    Err(_) => false,
                }
            }
        }
    } else {
        // Non-interactive: try root first, then user without prompts
        log::debug!("install (root, non-interactive) cmd: {}", root_cmd);
        match distro::enter_status(&selected.name, &root_cmd, true) {
            Ok(true) => true,
            _ => {
                log::debug!("install (user, non-interactive) fallback cmd: {}", user_cmd);
                match distro::enter_status(&selected.name, &user_cmd, false) {
                    Ok(true) => true,
                    _ => false,
                }
            }
        }
    };

    if !ok {
        // Re-run in non-interactive mode to capture error details
        // Capture diagnostics from both paths to provide helpful output
        let mut details = String::new();
        let diag_root = distro::enter_capture(&selected.name, &format!("{} 2>&1 || true", root_cmd), true);
        if let Ok(out) = diag_root { details.push_str(&String::from_utf8_lossy(&out.stdout)); details.push_str(&String::from_utf8_lossy(&out.stderr)); }
        let diag_user = distro::enter_capture(&selected.name, &format!("{} 2>&1 || true", user_cmd), false);
        if let Ok(out) = diag_user { details.push_str(&String::from_utf8_lossy(&out.stdout)); details.push_str(&String::from_utf8_lossy(&out.stderr)); }
        return Err(anyhow!("installation command failed inside container. Details:\n{}", details.trim()));
    }
    println!("Install completed.");
    if !cli.no_export {
        export_items(&selected.name, &bins, &apps)?;
        notify(&format!("Installed in {}", selected.name), &format!("Exported {} bins, {} apps", bins.len(), apps.len()));
    } else {
        println!("--no-export: skipping export stage");
    }
    Ok(())
}

fn doctor() -> Result<()> {
    println!("pkgbridge doctor:");

    // Check for distrobox
    match which::which("distrobox") {
        Ok(path) => println!("- distrobox: found at {}", path.display()),
        Err(_) => println!("- distrobox: NOT FOUND (install distrobox for full functionality)"),
    }

    // Check for container runtimes
    let podman = which::which("podman").is_ok();
    let docker = which::which("docker").is_ok();
    println!("- container runtime: podman: {}, docker: {}", yes_no(podman), yes_no(docker));

    // Check XDG dirs
    let home = std::env::var("HOME").unwrap_or_default();
    let bin_dir = std::env::var("XDG_BIN_HOME").ok().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{home}/.local/bin")));
    let apps_dir = std::env::var("XDG_DATA_HOME").ok().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{home}/.local/share"))).join("applications");
    println!("- bin dir: {}", bin_dir.display());
    println!("- applications dir: {}", apps_dir.display());
    // Writable checks
    println!("- bin dir writable: {}", yes_no(is_writable(&bin_dir)));
    println!("- applications dir writable: {}", yes_no(is_writable(&apps_dir)));
    // PATH contains bin dir
    println!("- bin dir on PATH: {}", yes_no(path_contains(&bin_dir)));
    // distrobox-export presence
    match which::which("distrobox-export") {
        Ok(path) => println!("- distrobox-export: found at {}", path.display()),
        Err(_) => println!("- distrobox-export: NOT FOUND (install distrobox-export for host integration)"),
    }
    // xdg-mime helper
    let have_xdg_mime = which::which("xdg-mime").is_ok();
    println!("- xdg-mime present: {}", yes_no(have_xdg_mime));
    let have_update_db = which::which("update-desktop-database").is_ok();
    println!("- update-desktop-database present: {}", yes_no(have_update_db));

    Ok(())
}

fn yes_no(b: bool) -> &'static str { if b { "yes" } else { "no" } }

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

fn init_logger(level: Option<LogLevel>) {
    let filter = match level.unwrap_or(LogLevel::Info) {
        LogLevel::Trace => log::LevelFilter::Trace,
        LogLevel::Debug => log::LevelFilter::Debug,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Error => log::LevelFilter::Error,
    };
    let mut builder = env_logger::Builder::new();
    builder.filter_level(filter);
    let _ = builder.try_init();
}

fn is_writable(p: &PathBuf) -> bool {
    std::fs::create_dir_all(p).ok();
    let test = p.join(".probe");
    match std::fs::OpenOptions::new().create(true).append(true).open(&test) {
        Ok(_) => { let _ = std::fs::remove_file(&test); true },
        Err(_) => false,
    }
}

fn path_contains(dir: &PathBuf) -> bool {
    std::env::var_os("PATH")
        .and_then(|v| v.into_string().ok())
        .map(|p| p.split(':').any(|s| s == dir.to_string_lossy()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FamilyArg { Debian, Fedora, Opensuse, Arch }

fn format_family(f: BoxFamily) -> &'static str {
    match f { BoxFamily::Debian => "debian", BoxFamily::Fedora => "fedora", BoxFamily::OpenSuse => "opensuse", BoxFamily::Arch => "arch" }
}

fn to_family(arg: FamilyArg) -> BoxFamily {
    match arg { FamilyArg::Debian => BoxFamily::Debian, FamilyArg::Fedora => BoxFamily::Fedora, FamilyArg::Opensuse => BoxFamily::OpenSuse, FamilyArg::Arch => BoxFamily::Arch }
}

struct SelectedBox {
    name: String,
    family: BoxFamily,
}

fn select_or_create(boxes: &[distro::DistroBox], fmt: &PackageFormat, cli: &Cli) -> Result<SelectedBox> {
    // If specific container requested, verify and classify
    if let Some(ref name) = cli.container {
        if !boxes.iter().any(|b| &b.name == name) {
            return Err(anyhow!("container '{}' not found", name));
        }
        let fam = distro::classify_box_family(name).context("classifying requested container")?;
        return Ok(SelectedBox { name: name.clone(), family: fam });
    }

    // Desired families based on format or explicit family
    let target_families: Vec<BoxFamily> = if let Some(fa) = cli.family {
        vec![to_family(fa)]
    } else {
        match fmt { PackageFormat::Deb => vec![BoxFamily::Debian], PackageFormat::Rpm => vec![BoxFamily::Fedora, BoxFamily::OpenSuse] }
    };

    // Try to find matching boxes
    let mut matches: Vec<(String, BoxFamily)> = Vec::new();
    for b in boxes {
        if let Ok(fam) = distro::classify_box_family(&b.name) {
            if target_families.contains(&fam) {
                matches.push((b.name.clone(), fam));
            }
        }
    }
    if matches.len() == 1 {
        let (name, fam) = matches.remove(0);
        return Ok(SelectedBox { name, family: fam });
    } else if matches.len() > 1 && std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
        println!("Multiple matching boxes found:");
        for (i, (name, fam)) in matches.iter().enumerate() {
            println!("  [{}] {} ({})", i + 1, name, format_family(*fam));
        }
        print!("Select a box [1-{}] or 0 to create new: ", matches.len());
        use std::io::Write; let _ = std::io::stdout().flush();
        let mut buf = String::new(); let _ = std::io::stdin().read_line(&mut buf);
        if let Ok(choice) = buf.trim().parse::<usize>() {
            if choice >= 1 && choice <= matches.len() {
                let (name, fam) = matches[choice - 1].clone();
                return Ok(SelectedBox { name, family: fam });
            }
        }
        // Fall through to creation if requested
    } else if matches.len() == 1 {
        let (name, fam) = matches.remove(0);
        return Ok(SelectedBox { name, family: fam });
    }

    // None found: create interactively or via --create
    let fam = target_families[0];
    let (default_name, default_image) = default_box_for_family(fam);
    let chosen_image = cli.create_image.clone().unwrap_or_else(|| default_image.to_string());
    if cli.create {
        println!("No matching box found. Creating '{}' from '{}'...", default_name, chosen_image);
        distro::create_box(default_name, &chosen_image)?;
        return Ok(SelectedBox { name: default_name.to_string(), family: fam });
    }
    // If interactive TTY, offer to create automatically
    if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
        println!("No matching box found for family '{}'.", format_family(fam));
        println!("I can create '{}' from '{}' now.", default_name, chosen_image);
        print!("Create it? [Y/n] ");
        use std::io::Write; let _ = std::io::stdout().flush();
        let mut buf = String::new(); let _ = std::io::stdin().read_line(&mut buf);
        let ans = buf.trim().to_ascii_lowercase();
        if ans.is_empty() || ans == "y" || ans == "yes" {
            distro::create_box(default_name, &chosen_image)?;
            return Ok(SelectedBox { name: default_name.to_string(), family: fam });
        }
    }

    Err(anyhow!("no matching box found; re-run with --create, pass --container/--family, or answer 'Y' when prompted"))
}

fn default_box_for_family(f: BoxFamily) -> (&'static str, &'static str) {
    match f {
        BoxFamily::Debian => ("debian-stable", "docker.io/library/debian:stable"),
        BoxFamily::Fedora => ("fedora-latest", "registry.fedoraproject.org/fedora:latest"),
        BoxFamily::OpenSuse => ("opensuse-tumbleweed", "registry.opensuse.org/opensuse/tumbleweed:latest"),
        BoxFamily::Arch => ("arch", "docker.io/library/archlinux:latest"),
    }
}

fn build_install_cmd_root(fmt: &PackageFormat, path: &str) -> String {
    // Root-mode installer: robust for local files and resolves dependencies
    // - Debian: dpkg -i file, then apt/apt-get -f install
    // - RPM families: dnf/zypper install the file directly
    let p = shell_escape::escape(std::borrow::Cow::from(path.to_string()));
    match fmt {
        PackageFormat::Deb => {
            // Use apt-get/apt to update indexes first, then dpkg -i and fix deps
            // Ensure we don't stop on the dpkg failure that triggers dependency fixing
            format!(
                "set -e; \
                 if command -v apt-get >/dev/null; then apt-get -y update; elif command -v apt >/dev/null; then apt -y update; fi; \
                 dpkg -i {} || {{ \
                   if command -v apt-get >/dev/null; then apt-get -y -f install; \
                   elif command -v apt >/dev/null; then apt -y -f install; \
                   else printf %s\\n apt-not-found >&2; exit 1; fi; \
                 }}",
                p
            )
        }
        PackageFormat::Rpm => {
            format!(
                "set -e; \
                 if command -v dnf >/dev/null; then dnf -y install {}; \
                 elif command -v zypper >/dev/null; then zypper --non-interactive install {}; \
                 else rpm -i {}; fi",
                p, p, p
            )
        }
    }
}

fn build_install_cmd_user(fmt: &PackageFormat, path: &str) -> String {
    let p = shell_escape::escape(std::borrow::Cow::from(path.to_string()));
    let inner = match fmt {
        PackageFormat::Deb => format!(
            "set -e; if command -v apt-get >/dev/null; then apt-get -y update; elif command -v apt >/dev/null; then apt -y update; fi; dpkg -i {} || {{ if command -v apt-get >/dev/null; then apt-get -y -f install; elif command -v apt >/dev/null; then apt -y -f install; else true; fi; }}",
            p
        ),
        PackageFormat::Rpm => format!(
            "set -e; if command -v dnf >/dev/null; then dnf -y install {}; elif command -v zypper >/dev/null; then zypper --non-interactive install {}; else rpm -i {}; fi",
            p, p, p
        ),
    };
    // Prefer sudo (passwordless or interactive), then doas, else run without elevation (may fail)
    format!(
        "set -e; if command -v sudo >/dev/null; then if sudo -n true >/dev/null 2>&1; then sudo sh -lc '{}' ; else sudo sh -lc '{}' ; fi; elif command -v doas >/dev/null; then doas sh -lc '{}' ; else sh -lc '{}' ; fi",
        inner, inner, inner, inner
    )
}

fn build_install_cmd_user_noninteractive(fmt: &PackageFormat, path: &str) -> String {
    let p = shell_escape::escape(std::borrow::Cow::from(path.to_string()));
    let inner = match fmt {
        PackageFormat::Deb => format!(
            "set -e; if command -v apt-get >/dev/null; then apt-get -y update; elif command -v apt >/dev/null; then apt -y update; fi; dpkg -i {} || {{ if command -v apt-get >/dev/null; then apt-get -y -f install; elif command -v apt >/dev/null; then apt -y -f install; else true; fi; }}",
            p
        ),
        PackageFormat::Rpm => format!(
            "set -e; if command -v dnf >/dev/null; then dnf -y install {}; elif command -v zypper >/dev/null; then zypper --non-interactive install {}; else rpm -i {}; fi",
            p, p, p
        ),
    };
    // Force non-interactive sudo so we can capture errors, even if it fails due to needing a password
    format!(
        "set -e; if command -v sudo >/dev/null; then sudo -n sh -lc '{}' ; elif command -v doas >/dev/null; then doas sh -lc '{}' ; else sh -lc '{}' ; fi",
        inner, inner, inner
    )
}

fn prescan_package(box_name: &str, fmt: &PackageFormat, in_box_path: &str) -> Result<(Vec<String>, Vec<String>)> {
    let cmd = match fmt {
        PackageFormat::Deb => format!("dpkg -c {} || true", shell_escape::escape(std::borrow::Cow::from(in_box_path.to_string()))),
        PackageFormat::Rpm => format!("rpm -qlp {} || true", shell_escape::escape(std::borrow::Cow::from(in_box_path.to_string()))),
    };
    let out = distro::enter_capture(box_name, &cmd, false)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut bins = Vec::new();
    let mut apps = Vec::new();
    match fmt {
        PackageFormat::Deb => {
            for line in stdout.lines() {
                // dpkg -c lines end with the path, often prefixed by ./
                if let Some(idx) = line.rfind(' ') {
                    let mut path = line[idx..].trim().to_string();
                    if let Some(stripped) = path.strip_prefix('.') { path = stripped.to_string(); }
                    if let Some(stripped) = path.strip_prefix('/') { path = stripped.to_string(); }
                    if let Some(name) = path.strip_prefix("usr/bin/") {
                        if !name.is_empty() && !name.ends_with('/') { bins.push(name.to_string()); }
                    }
                    if let Some(rest) = path.strip_prefix("usr/share/applications/") {
                        if rest.ends_with(".desktop") { apps.push(rest.to_string()); }
                    }
                }
            }
        }
        PackageFormat::Rpm => {
            for mut path in stdout.lines().map(|s| s.trim().to_string()) {
                if let Some(stripped) = path.strip_prefix('/') { path = stripped.to_string(); }
                if let Some(name) = path.strip_prefix("usr/bin/") {
                    if !name.is_empty() && !name.ends_with('/') { bins.push(name.to_string()); }
                }
                if let Some(rest) = path.strip_prefix("usr/share/applications/") {
                    if rest.ends_with(".desktop") { apps.push(rest.to_string()); }
                }
            }
        }
    }
    bins.sort(); bins.dedup();
    apps.sort(); apps.dedup();
    Ok((bins, apps))
}

fn export_items(box_name: &str, bins: &[String], apps: &[String]) -> Result<()> {
    if bins.is_empty() && apps.is_empty() {
        println!("No items detected to export. You can pass --bin or --app.");
        return Ok(());
    }
    let bin_dir = host_bin_dir();
    for b in bins {
        // Pre-check for collision
        let target = bin_dir.join(b);
        if target.exists() {
            // Fall back to custom shim with -<container> suffix
            let alt = format!("{}-{}", b, box_name);
            write_simple_shim(&bin_dir, &alt, box_name, b)?;
            println!("Name collision for '{}'; exported as '{}'", b, alt);
            continue;
        }
        if export_bin(box_name, b) {
            println!("Exported bin: {}", b);
        } else {
            // Try custom shim as fallback
            let _ = write_simple_shim(&bin_dir, b, box_name, b);
            eprintln!("Warning: distrobox-export failed; wrote shim for {}", b);
        }
    }
    let apps_dir = host_apps_dir();
    for app in apps {
        // use basename for app exporting when possible
        let base = std::path::Path::new(app).file_name().and_then(|s| s.to_str()).unwrap_or(app);
        let target = apps_dir.join(base);
        if target.exists() {
            // Collision; copy with container suffix and rewrite Exec
            let in_path = format!("/usr/share/applications/{}", base);
            let out = distro::enter_capture(box_name, &format!("cat {}", shell_escape::escape(std::borrow::Cow::from(in_path.clone()))), false)?;
            let mut content = String::from_utf8_lossy(&out.stdout).to_string();
            // Rewrite Exec lines
            let mut new_lines = Vec::new();
            for line in content.lines() {
                if line.starts_with("Exec=") && !line.contains("distrobox enter -n") {
                    let old = &line[5..];
                    let replaced = format!("Exec=distrobox enter -n {} -- {}", box_name, old);
                    new_lines.push(replaced);
                } else {
                    new_lines.push(line.to_string());
                }
            }
            let new_content = new_lines.join("\n");
            let alt_name = format!("{}.{}.desktop", base.trim_end_matches(".desktop"), box_name);
            std::fs::create_dir_all(&apps_dir).ok();
            std::fs::write(apps_dir.join(&alt_name), new_content)?;
            println!("App collision for '{}'; exported as '{}'", base, alt_name);
            continue;
        }
        // For export, prefer absolute path when we know it's a desktop file
        let export_target = if app.contains('/') || base.ends_with(".desktop") {
            if app.starts_with('/') { app.to_string() } else { format!("/usr/share/applications/{}", base) }
        } else { base.to_string() };
        if export_app(box_name, &export_target) {
            println!("Exported app: {}", base);
        } else {
            eprintln!("Warning: failed exporting app {}", base);
        }
    }
    Ok(())
}

fn host_bin_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::env::var("XDG_BIN_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(format!("{home}/.local/bin")))
}

fn host_apps_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(format!("{home}/.local/share"))).join("applications")
}

fn write_simple_shim(dir: &std::path::Path, out_name: &str, box_name: &str, cmd_name: &str) -> Result<()> {
    let path = dir.join(out_name);
    let content = format!("#!/usr/bin/env sh\nexec distrobox enter -n {} -- {} \"$@\"\n", box_name, cmd_name);
    std::fs::create_dir_all(dir).ok();
    std::fs::write(&path, content)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

fn dbe_supports_container_flag() -> bool {
    if which::which("distrobox-export").is_err() { return false; }
    match std::process::Command::new("distrobox-export").arg("--help").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains("--container"),
        Err(_) => false,
    }
}

fn export_bin(box_name: &str, bin: &str) -> bool {
    let supports = dbe_supports_container_flag();
    if supports {
        // Try by name first, then fallback to absolute path
        let status = std::process::Command::new("distrobox-export")
            .args(["--container", box_name, "--bin", bin])
            .status();
        if let Ok(s) = status { if s.success() { return true; } }
        let abs = format!("/usr/bin/{}", bin);
        let status2 = std::process::Command::new("distrobox-export")
            .args(["--container", box_name, "--bin", &abs])
            .status();
        return matches!(status2, Ok(s) if s.success());
    } else {
        // Older versions: run from inside container, requires absolute path
        let abs = format!("/usr/bin/{}", bin);
        let status = std::process::Command::new("distrobox")
            .args(["enter", "-n", box_name, "--", "distrobox-export", "--bin", &abs])
            .status();
        return matches!(status, Ok(s) if s.success());
    }
}

fn export_app(box_name: &str, app_spec: &str) -> bool {
    // Normalize: if caller passed only a basename ending with .desktop, use absolute path
    // Accepted by distrobox-export: app name (without .desktop) or absolute path to .desktop
    let normalized = if app_spec.contains('/') {
        app_spec.to_string()
    } else if app_spec.ends_with(".desktop") {
        format!("/usr/share/applications/{}", app_spec)
    } else {
        app_spec.to_string()
    };
    let supports = dbe_supports_container_flag();
    if supports {
        let status = std::process::Command::new("distrobox-export")
            .args(["--container", box_name, "--app", &normalized])
            .status();
        return matches!(status, Ok(s) if s.success());
    } else {
        let status = std::process::Command::new("distrobox")
            .args(["enter", "-n", box_name, "--", "distrobox-export", "--app", &normalized])
            .status();
        return matches!(status, Ok(s) if s.success());
    }
}

fn scan_installed_pkg(box_name: &str, fam: BoxFamily, pkg: &str) -> Result<(Vec<String>, Vec<String>)> {
    let cmd = match fam {
        BoxFamily::Debian => format!("dpkg -L {}", shell_escape::escape(std::borrow::Cow::from(pkg.to_string()))),
        BoxFamily::Fedora | BoxFamily::OpenSuse | BoxFamily::Arch => format!("rpm -ql {}", shell_escape::escape(std::borrow::Cow::from(pkg.to_string()))),
    };
    let out = distro::enter_capture(box_name, &cmd, false)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut bins = Vec::new();
    let mut apps = Vec::new();
    for mut path in stdout.lines().map(|s| s.trim().to_string()) {
        if let Some(stripped) = path.strip_prefix('/') { path = stripped.to_string(); }
        if let Some(name) = path.strip_prefix("usr/bin/") { if !name.is_empty() && !name.ends_with('/') { bins.push(name.to_string()); } }
        if let Some(rest) = path.strip_prefix("usr/share/applications/") { if rest.ends_with(".desktop") { apps.push(rest.to_string()); } }
    }
    bins.sort(); bins.dedup();
    apps.sort(); apps.dedup();
    Ok((bins, apps))
}

fn export_pkg(cli: &Cli, arg: PkgArg) -> Result<()> {
    let name = cli.container.clone().ok_or_else(|| anyhow!("--container is required for export"))?;
    let fam = distro::classify_box_family(&name)?;
    let (mut bins, mut apps) = scan_installed_pkg(&name, fam, &arg.pkg)?;
    if !cli.bin.is_empty() { bins = cli.bin.clone(); }
    if !cli.app.is_empty() { apps = cli.app.clone(); }
    if cli.dry_run {
        println!("--dry-run: would export bins={:?}, apps={:?}", bins, apps);
        return Ok(());
    }
    export_items(&name, &bins, &apps)
}

fn uninstall_pkg(cli: &Cli, arg: PkgArg) -> Result<()> {
    let name = cli.container.clone().ok_or_else(|| anyhow!("--container is required for uninstall"))?;
    let fam = distro::classify_box_family(&name)?;
    let (bins, apps) = scan_installed_pkg(&name, fam, &arg.pkg).unwrap_or_default();
    if !bins.is_empty() || !apps.is_empty() {
        println!("Removing exports for package '{}'...", arg.pkg);
        if !cli.dry_run { unexport_items(&name, &bins, &apps); }
    }
    let ok = uninstall_inside(&name, fam, &arg.pkg, cli.dry_run)?;
    if ok { println!("Uninstall completed."); } else { println!("Uninstall command reported failure."); }
    Ok(())
}

fn unexport_items(box_name: &str, bins: &[String], apps: &[String]) {
    let supports = dbe_supports_container_flag();
    for b in bins {
        if supports {
            let _ = std::process::Command::new("distrobox-export").args(["--container", box_name, "--delete", "--bin", b]).status();
        } else {
            // Older versions expect absolute path and to be run inside the container
            let abs = format!("/usr/bin/{}", b);
            let _ = std::process::Command::new("distrobox").args(["enter", "-n", box_name, "--", "distrobox-export", "--delete", "--bin", &abs]).status();
        }
    }
    for app in apps {
        let base = std::path::Path::new(app).file_name().and_then(|s| s.to_str()).unwrap_or(app);
        if supports {
            let _ = std::process::Command::new("distrobox-export").args(["--container", box_name, "--delete", "--app", base]).status();
        } else {
            let _ = std::process::Command::new("distrobox").args(["enter", "-n", box_name, "--", "distrobox-export", "--delete", "--app", base]).status();
        }
    }
}

fn uninstall_inside(box_name: &str, fam: BoxFamily, pkg: &str, dry_run: bool) -> Result<bool> {
    let p = shell_escape::escape(std::borrow::Cow::from(pkg.to_string()));
    let inner = match fam {
        BoxFamily::Debian => format!("set -e; if command -v apt-get >/dev/null; then apt-get -y remove {}; else dpkg -r {}; fi", p, p),
        BoxFamily::Fedora => format!("set -e; if command -v dnf >/dev/null; then dnf -y remove {}; else rpm -e {}; fi", p, p),
        BoxFamily::OpenSuse => format!("set -e; if command -v zypper >/dev/null; then zypper --non-interactive rm {}; else rpm -e {}; fi", p, p),
        BoxFamily::Arch => format!("set -e; if command -v pacman >/dev/null; then pacman -R --noconfirm {}; else echo 'pacman not found' >&2; exit 1; fi", p),
    };
    let cmd = format!(
        "set -e; if command -v sudo >/dev/null; then if sudo -n true >/dev/null 2>&1; then sudo sh -lc '{}' ; else sudo sh -lc '{}' ; fi; elif command -v doas >/dev/null; then doas sh -lc '{}' ; else sh -lc '{}' ; fi",
        inner, inner, inner, inner
    );
    if dry_run { println!("--dry-run: would run inside '{}': {}", box_name, cmd); return Ok(true); }
    distro::enter_status(box_name, &cmd, false)
}

fn preseed_password(box_name: &str, password: &str) -> Result<()> {
    // One-time, opt-in: set the user password via root so future sudo prompts work non-interactively too
    // Determine a likely non-root user from /etc/passwd (first uid >= 1000), then set its password via chpasswd
    return preseed_password_root(box_name, password);
    /*
    let pw = shell_escape::escape(std::borrow::Cow::from(password.to_string()));
    let cmd = format!(
        "u=$(awk -F: '$3>=1000 && $1!="nobody" {print $1; exit}' /etc/passwd); \
         if [ -z \"$u\" ]; then u=$(getent passwd 1000 | cut -d: -f1 || true); fi; \
         if [ -z \"$u\" ]; then u=$(getent passwd 1001 | cut -d: -f1 || true); fi; \
         if [ -z \"$u\" ]; then echo 'no non-root user found' >&2; exit 1; fi; \
         printf '%s:%s\n' \"$u\" {} | chpasswd",
        pw
    );
    let ok = distro::enter_status(box_name, &cmd, true).context("setting initial password via chpasswd (root)")?;
    if !ok { return Err(anyhow!("failed to set initial password in container")); }
    */
    Ok(())
}

fn preseed_password_root(box_name: &str, password: &str) -> Result<()> {
    // Opt-in: set the password for the first non-system user (uid>=1000) via chpasswd as root.
    let pw = shell_escape::escape(std::borrow::Cow::from(password.to_string()));
    // Verify we can enter as root; if not, skip silently
    if !distro::enter_status(box_name, "true", true).unwrap_or(false) {
        return Ok(());
    }
    let cmd = format!(r#"u=$(awk -F: '$3>=1000 && $1!="nobody" {{print $1; exit}}' /etc/passwd); \
         if [ -z "$u" ]; then u=$(getent passwd 1000 | cut -d: -f1 || true); fi; \
         if [ -z "$u" ]; then u=$(getent passwd 1001 | cut -d: -f1 || true); fi; \
         if [ -z "$u" ]; then echo 'no non-root user found' >&2; exit 1; fi; \
         printf '%s:%s\n' "$u" {} | chpasswd"#, pw);
    let ok = distro::enter_status(box_name, &cmd, true).context("setting initial password via chpasswd (root)")?;
    if !ok { return Err(anyhow!("failed to set initial password in container")); }
    Ok(())
}

fn pm_cmd(cmd: PmCmd) -> Result<()> {
    match cmd {
        PmCmd::SetDefault { family, box_name } => pm::set_default(to_family(family), &box_name),
        PmCmd::GenerateShims => pm::generate_shims(),
        PmCmd::ShowDefaults => {
            let map = pm::show_defaults();
            if map.is_empty() { println!("No defaults set."); } else { for (k, v) in map { println!("{} => {}", k, v); } }
            Ok(())
        }
        PmCmd::Snapshot => pm_snapshot(),
        PmCmd::PostTransaction => pm_post_transaction(),
    }
}

fn pm_snapshot() -> Result<()> {
    let name = std::env::args().collect::<Vec<_>>(); // container is passed via global --container
    // Resolve container and family
    let container = std::env::args().skip_while(|a| a != "--container").nth(1)
        .or_else(|| std::env::var("PKGBRIDGE_CONTAINER").ok())
        .ok_or_else(|| anyhow!("--container is required for pm snapshot"))?;
    let fam = std::env::args().skip_while(|a| a != "--family").nth(1)
        .or_else(|| Some(format_family(distro::classify_box_family(&container).ok()?)).map(|s| s.to_string()))
        .unwrap_or_else(|| "".into());
    let list = list_installed_pkgs(&container, None)?;
    std::fs::create_dir_all(crate::config::snapshot_dir()).ok();
    std::fs::write(crate::config::snapshot_path(&container), list.join("\n"))?;
    Ok(())
}

fn pm_post_transaction() -> Result<()> {
    let container = std::env::args().skip_while(|a| a != "--container").nth(1)
        .or_else(|| std::env::var("PKGBRIDGE_CONTAINER").ok())
        .ok_or_else(|| anyhow!("--container is required for pm post-transaction"))?;
    let fam = distro::classify_box_family(&container)?;
    let before = std::fs::read_to_string(crate::config::snapshot_path(&container)).unwrap_or_default();
    let before_set: std::collections::HashMap<String, String> = before.lines().filter_map(|l| {
        let mut sp = l.splitn(2, '\t');
        Some((sp.next()?.to_string(), sp.next().unwrap_or("").to_string()))
    }).collect();
    let after_list = list_installed_pkgs(&container, Some(fam))?;
    let mut after_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in &after_list { let mut sp = l.splitn(2, '\t'); if let (Some(n), Some(v)) = (sp.next(), sp.next()) { after_map.insert(n.to_string(), v.to_string()); } }
    let mut new_pkgs = Vec::new();
    let mut upgraded = Vec::new();
    for (name, ver) in &after_map {
        match before_set.get(name) {
            None => new_pkgs.push(name.clone()),
            Some(prev) if prev != ver => upgraded.push(name.clone()),
            _ => {}
        }
    }
    if new_pkgs.is_empty() && upgraded.is_empty() { return Ok(()); }
    log::info!("Detected new: {:?}, upgraded: {:?}", new_pkgs, upgraded);
    let mut pkgs: Vec<String> = new_pkgs;
    pkgs.extend(upgraded);
    for pkg in pkgs {
        let (bins, apps) = scan_installed_pkg(&container, fam, &pkg).unwrap_or_default();
        let _ = export_items(&container, &to_names_only(bins), &apps);
    }
    // Update snapshot to after state
    std::fs::write(crate::config::snapshot_path(&container), after_list.join("\n"))?;
    Ok(())
}

fn list_installed_pkgs(container: &str, fam: Option<BoxFamily>) -> Result<Vec<String>> {
    let fam = fam.unwrap_or(distro::classify_box_family(container)?);
    let cmd = match fam {
        BoxFamily::Debian => "dpkg-query -W -f='${Package}\t${Version}\n'".to_string(),
        BoxFamily::Fedora | BoxFamily::OpenSuse | BoxFamily::Arch => "rpm -qa --qf '%{NAME}\t%{VERSION}-%{RELEASE}\n'".to_string(),
    };
    let out = distro::enter_capture(container, &cmd, false)?;
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

fn to_names_only(bins: Vec<String>) -> Vec<String> { bins }

fn desktop_cmd(cmd: DesktopCmd, dry_run: bool) -> Result<()> {
    match cmd {
        DesktopCmd::Install => desktop::install(dry_run),
        DesktopCmd::Uninstall => desktop::uninstall(dry_run),
    }
}

fn notify(summary: &str, body: &str) {
    if which::which("notify-send").is_ok() {
        let _ = std::process::Command::new("notify-send").args([summary, body]).status();
    }
}

fn maybe_first_run_prompt() {
    use std::io::{self, Write};
    let mut st = config::load_state();
    if st.first_run_done { return; }
    // Only prompt in interactive terminals
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        // Defer onboarding to first interactive run
        return;
    }
    let boxes = match distro::discover_boxes() { Ok(b) => b, Err(_) => vec![] };
    if boxes.is_empty() { st.first_run_done = true; let _ = config::save_state(&st); return; }
    // Determine families present and select first box per family for defaults
    let mut fam_to_box: std::collections::HashMap<BoxFamily, String> = std::collections::HashMap::new();
    let mut total_apps = 0usize;
    for b in &boxes {
        if let Ok(fam) = distro::classify_box_family(&b.name) {
            fam_to_box.entry(fam).or_insert(b.name.clone());
            // Count apps
            let out = distro::enter_capture(&b.name, "ls -1 /usr/share/applications/*.desktop 2>/dev/null | wc -l", false);
            if let Ok(out) = out {
                if let Ok(s) = String::from_utf8(out.stdout) { total_apps += s.trim().parse::<usize>().unwrap_or(0); }
            }
        }
    }
    if fam_to_box.is_empty() && total_apps == 0 { st.first_run_done = true; let _ = config::save_state(&st); return; }
    println!("pkgbridge first-run setup:");
    let fam_list: Vec<&'static str> = fam_to_box.keys().map(|&f| format_family(f)).collect();
    if !fam_list.is_empty() { println!("- Found families: {}", fam_list.join(", ")); }
    if total_apps > 0 { println!("- Found ~{} desktop apps across boxes", total_apps); }
    print!("Generate package-manager shims and export existing desktop apps now? [Y/n] ");
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    let ans = buf.trim().to_ascii_lowercase();
    if ans.is_empty() || ans == "y" || ans == "yes" {
        // Save defaults from fam_to_box
        let mut cfg = config::load_config();
        for (fam, bx) in fam_to_box.iter() { cfg.pm_defaults.insert(pm::family_key(*fam).into(), bx.clone()); }
        let _ = config::save_config(&cfg);
        // Generate shims
        let _ = pm::generate_shims();
        // Export apps
        for b in &boxes {
            // Enumerate apps and export
            if let Ok(out) = distro::enter_capture(&b.name, "ls -1 /usr/share/applications/*.desktop 2>/dev/null", false) {
                if out.status.success() {
                    if let Ok(s) = String::from_utf8(out.stdout) {
                        for line in s.lines() {
                            let base = std::path::Path::new(line.trim()).file_name().and_then(|x| x.to_str()).unwrap_or("");
                            if base.is_empty() { continue; }
                            let _ = export_app(&b.name, base);
                        }
                    }
                }
            }
        }
        println!("First-run export completed.");
    }
    // Also ensure bootstrap shims exist for missing host managers (apt/dnf/zypper/pacman)
    let _ = pm::generate_bootstrap_shims();
    st.first_run_done = true; let _ = config::save_state(&st);
}
