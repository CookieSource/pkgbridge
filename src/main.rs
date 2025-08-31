mod cli;
mod pkgdetect;
mod distro;
mod pm;
mod config;
mod desktop;

use anyhow::Result;

fn main() -> Result<()> {
    cli::run()
}
