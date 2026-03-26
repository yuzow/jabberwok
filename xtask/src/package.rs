use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::Platform;

pub fn package(platform: Platform) -> Result<()> {
    ensure_host_matches(platform)?;
    build_release_binary()?;

    match platform {
        Platform::Macos => crate::macos::package(),
        Platform::Windows => crate::windows::package(),
        Platform::Linux => crate::linux::package(),
    }
}

fn ensure_host_matches(platform: Platform) -> Result<()> {
    let host = std::env::consts::OS;
    let target = platform_name(platform);

    if host == target {
        Ok(())
    } else {
        bail!("packaging for {target} must be run on a {target} host; current host is {host}")
    }
}

fn build_release_binary() -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--bin", "jabberwok"])
        .status()
        .context("failed to run cargo build --release --bin jabberwok")?;

    if status.success() {
        Ok(())
    } else {
        bail!("cargo build --release --bin jabberwok failed")
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        Platform::Linux => "linux",
    }
}
