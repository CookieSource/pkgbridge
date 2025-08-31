use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default)]
pub struct DistroBox {
    pub name: String,
    pub image: Option<String>,
    pub runtime: String, // podman/docker/unknown
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Debian,
    Fedora,
    OpenSuse,
    Arch,
}

/// Try to discover existing Distrobox containers.
/// - First, attempt `distrobox list --json` and parse it.
/// - Fallback to `distrobox list` and attempt simple parsing.
pub fn discover_boxes() -> Result<Vec<DistroBox>> {
    // Try JSON mode first
    let json_out = Command::new("distrobox")
        .arg("list")
        .arg("--json")
        .output();

    if let Ok(out) = json_out {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !stdout.trim().is_empty() {
                if let Ok(list) = parse_boxes_json(stdout.as_ref()) {
                    return Ok(list);
                }
            }
        }
    }

    // Fallback to plain text
    let out = Command::new("distrobox").arg("list").output().with_context(|| "running 'distrobox list'")?;
    if !out.status.success() {
        // Not found or error; return empty gracefully
        return Ok(vec![]);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_boxes_plain(stdout.as_ref()))
}

#[derive(Debug, Deserialize)]
struct JsonBox {
    name: String,
    image: Option<String>,
    #[serde(default)]
    engine: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonList {
    containers: Vec<JsonBox>,
}

fn parse_boxes_json(s: &str) -> Result<Vec<DistroBox>> {
    // Accept either a top-level array or an object with containers
    if s.trim_start().starts_with('[') {
        let arr: Vec<JsonBox> = serde_json::from_str(s)?;
        Ok(arr
            .into_iter()
            .map(|j| DistroBox { name: j.name, image: j.image, runtime: j.engine.unwrap_or_else(|| "unknown".into()) })
            .collect())
    } else {
        let obj: JsonList = serde_json::from_str(s)?;
        Ok(obj
            .containers
            .into_iter()
            .map(|j| DistroBox { name: j.name, image: j.image, runtime: j.engine.unwrap_or_else(|| "unknown".into()) })
            .collect())
    }
}

fn parse_boxes_plain(s: &str) -> Vec<DistroBox> {
    // Handle two common formats:
    // 1) Pipe table: "ID | NAME | STATUS | IMAGE"
    // 2) Space-separated name and image (very old versions)
    let mut boxes = Vec::new();
    let mut saw_pipe_header = false;
    for line in s.lines() {
        let t = line.trim();
        if t.is_empty() { continue; }

        if t.contains('|') {
            // Pipe-separated table
            // Split and trim columns
            let mut cols: Vec<String> = t.split('|').map(|c| c.trim().to_string()).collect();
            // Skip header row
            if cols.iter().any(|c| c.eq_ignore_ascii_case("NAME")) && cols.iter().any(|c| c.eq_ignore_ascii_case("ID")) {
                saw_pipe_header = true;
                continue;
            }
            // Skip separators like "+---" if present
            if cols.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == '+')) { continue; }
            if cols.len() >= 2 {
                let name = cols.get(1).cloned().unwrap_or_default();
                if name.is_empty() || name.eq_ignore_ascii_case("NAME") { continue; }
                let image = cols.get(3).cloned();
                boxes.push(DistroBox { name, image, runtime: "unknown".into() });
                continue;
            }
        }

        // Fallback heuristic for plain whitespace formats
        if t.starts_with("NAME") || t.starts_with("+---") || t.contains("CONTAINER ID") || t.eq_ignore_ascii_case("id") {
            continue;
        }
        let parts: Vec<&str> = t.split_whitespace().collect();
        if parts.len() >= 1 {
            // If we saw a pipe header earlier, the first column here is likely ID; skip such lines
            if saw_pipe_header && parts.get(0).map(|c| c.len()).unwrap_or(0) >= 6 && parts.get(1).is_some() {
                // Likely an ID then NAME; take NAME
                let name = parts.get(1).unwrap().to_string();
                let image = parts.get(3).map(|s| s.to_string());
                boxes.push(DistroBox { name, image, runtime: "unknown".into() });
            } else {
                let name = parts[0].to_string();
                if name.eq_ignore_ascii_case("NAME") || name.eq_ignore_ascii_case("Created") { continue; }
                let image = parts.get(1).map(|s| s.to_string());
                boxes.push(DistroBox { name, image, runtime: "unknown".into() });
            }
        }
    }
    boxes
}

/// Classify a Distrobox into a Linux distribution family by reading /etc/os-release inside it.
pub fn classify_box_family(name: &str) -> Result<Family> {
    let out = Command::new("distrobox")
        .args(["enter", "-n", name, "--", "sh", "-lc", "cat /etc/os-release 2>/dev/null || true"])
        .output()
        .with_context(|| format!("running 'distrobox enter' for {name}"))?;
    if !out.status.success() {
        return Err(anyhow!("failed to enter box {name} to read /etc/os-release"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (id, id_like) = parse_os_release(text.as_ref());
    classify_ids(&id, &id_like).ok_or_else(|| anyhow!("could not classify family for box {name}"))
}

fn parse_os_release(s: &str) -> (Option<String>, Vec<String>) {
    let mut id: Option<String> = None;
    let mut id_like: Vec<String> = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(rest) = line.strip_prefix("ID=") {
            id = Some(unquote(rest).to_ascii_lowercase());
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            let raw = unquote(rest).to_ascii_lowercase();
            id_like.extend(raw.split_whitespace().map(|t| t.to_string()));
        }
    }
    (id, id_like)
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        t[1..t.len().saturating_sub(1)].to_string()
    } else { t.to_string() }
}

fn classify_ids(id: &Option<String>, id_like: &Vec<String>) -> Option<Family> {
    let mut tokens: Vec<String> = Vec::new();
    if let Some(i) = id { tokens.push(i.clone()); }
    tokens.extend(id_like.clone());
    let has = |k: &str| tokens.iter().any(|t| t == k);
    if has("debian") || has("ubuntu") { return Some(Family::Debian); }
    if has("fedora") || has("rhel") || has("centos") { return Some(Family::Fedora); }
    if has("opensuse") || has("sles") || has("suse") { return Some(Family::OpenSuse); }
    if has("arch") || has("manjaro") || has("endeavouros") { return Some(Family::Arch); }
    None
}

/// Create a distrobox with the given name and image.
pub fn create_box(name: &str, image: &str) -> Result<()> {
    let status = Command::new("distrobox")
        .args(["create", "--name", name, "--image", image, "-Y", "--yes"]) // accept both variants
        .status()
        .with_context(|| format!("creating distrobox {name} from {image}"))?;
    if !status.success() {
        return Err(anyhow!("distrobox create failed for {name}"));
    }
    Ok(())
}

/// Run a command inside a distrobox and capture output
pub fn enter_capture(name: &str, cmd: &str, as_root: bool) -> Result<std::process::Output> {
    let mut c = Command::new("distrobox");
    c.arg("enter");
    if as_root { c.arg("--root"); }
    c.args(["-n", name, "--", "sh", "-lc", cmd]);
    let out = c.output().with_context(|| format!("entering box {name} to run: {cmd}"))?;
    Ok(out)
}

/// Run a command inside a distrobox and return exit status only
pub fn enter_status(name: &str, cmd: &str, as_root: bool) -> Result<bool> {
    let out = enter_capture(name, cmd, as_root)?;
    Ok(out.status.success())
}

/// Copy a local file into the box at /tmp/pkgbridge/<sanitized-basename> via stdin piping.
/// Returns the destination path inside the container.
pub fn copy_into_box(name: &str, local_path: &std::path::Path) -> Result<String> {
    let data = std::fs::read(local_path).with_context(|| format!("reading {}", local_path.display()))?;
    let base = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("package");
    let mut sanitized = String::new();
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' { sanitized.push(ch); } else { sanitized.push('_'); }
    }
    if sanitized.is_empty() { sanitized.push_str("package"); }
    let dest = format!("/tmp/pkgbridge/{sanitized}");
    let quoted = shell_escape::escape(std::borrow::Cow::from(dest.clone()));
    let cmd = format!("mkdir -p /tmp/pkgbridge && cat > {quoted}");

    let mut child = Command::new("distrobox")
        .arg("enter")
        .arg("-n").arg(name)
        .args(["--", "sh", "-lc", &cmd])
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning distrobox enter for copy into {name}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("failed to open stdin to container"))?
        .write_all(&data)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow!("copy into container failed"));
    }
    Ok(dest)
}
