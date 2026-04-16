use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{MacosPackageStage, Platform};

pub fn package(platform: Platform) -> Result<()> {
    ensure_host_matches(platform)?;

    match platform {
        Platform::Macos => {
            build_release_binary()?;
            crate::macos::package()
        }
        Platform::Windows => {
            build_release_binary()?;
            crate::windows::package()
        }
        Platform::Linux => {
            build_release_binary()?;
            crate::linux::package()
        }
    }
}

pub fn package_macos(stage: MacosPackageStage) -> Result<()> {
    ensure_host_matches(Platform::Macos)?;

    match stage {
        MacosPackageStage::BuildBinary => build_release_binary(),
        MacosPackageStage::StageApp => {
            build_release_binary()?;
            crate::macos::stage_app_bundle()?;
            Ok(())
        }
        MacosPackageStage::PackageDmg => {
            crate::macos::package_dmg()?;
            Ok(())
        }
        MacosPackageStage::All => {
            build_release_binary()?;
            crate::macos::package()
        }
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

pub fn build_release_binary() -> Result<()> {
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
