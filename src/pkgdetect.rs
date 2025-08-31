use std::{fs::File, io::{Read, Seek, SeekFrom}, path::Path};

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFormat { Deb, Rpm }

pub fn detect_package_format(path: &Path) -> Result<PackageFormat> {
    // Extension hint first
    if let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) {
        match ext.as_str() {
            "deb" => return Ok(PackageFormat::Deb),
            "rpm" => return Ok(PackageFormat::Rpm),
            _ => {}
        }
    }

    // Fallback: sniff magic
    let mut f = File::open(path)?;
    let mut header = [0u8; 8];
    let n = f.read(&mut header)?;
    if n >= 8 {
        // RPM lead magic: 0xed 0xab 0xee 0xdb
        if &header[0..4] == [0xed, 0xab, 0xee, 0xdb] { return Ok(PackageFormat::Rpm); }
        // ar archive magic for .deb: "!<arch>\n"
        if &header[0..8] == *b"!<arch>\n" {
            // further check for debian-binary member (best-effort quick scan)
            // Not reading whole ar; just a hint that this is likely a .deb
            return Ok(PackageFormat::Deb);
        }
    }

    // Last resort: look for "debian-binary" somewhere near the beginning
    f.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; 512 * 4];
    let n = f.read(&mut buf)?;
    let hay = &buf[..n];
    if hay.windows(13).any(|w| w == b"debian-binary") { return Ok(PackageFormat::Deb); }

    Err(anyhow!("unknown package format for {}", path.display()))
}

