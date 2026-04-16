use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

const ROOT_CARGO_TOML: &str = "Cargo.toml";
const ROOT_CARGO_LOCK: &str = "Cargo.lock";

pub fn prepare_release(version: &str, push: bool) -> Result<()> {
    validate_version(version)?;
    ensure_git_clean()?;

    let cargo_toml_path = Path::new(ROOT_CARGO_TOML);
    let current_manifest = fs::read_to_string(cargo_toml_path)
        .with_context(|| format!("failed to read {}", cargo_toml_path.display()))?;
    let current_version = package_version(&current_manifest)?;

    if current_version == version {
        bail!("Cargo.toml is already at version {version}");
    }

    ensure_tag_absent(version)?;

    let updated_manifest = replace_package_version(&current_manifest, version)?;
    fs::write(cargo_toml_path, updated_manifest)
        .with_context(|| format!("failed to write {}", cargo_toml_path.display()))?;

    if let Err(err) = update_lockfile() {
        let _ = fs::write(cargo_toml_path, current_manifest);
        return Err(err);
    }

    if let Err(err) = commit_and_tag(version) {
        let _ = restore_release_files(&current_manifest);
        return Err(err);
    }

    let branch = current_branch()?;

    if push {
        push_release(&branch, version)?;
    }

    println!("prepared release v{version}");
    if push {
        println!("pushed {branch} and v{version} to origin");
    } else {
        println!("next: git push origin {branch} && git push origin v{version}");
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<()> {
    let (core, prerelease) = match version.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (version, None),
    };

    validate_core_version(core)?;

    if let Some(prerelease) = prerelease {
        validate_prerelease(prerelease)?;
    }

    Ok(())
}

fn validate_core_version(core: &str) -> Result<()> {
    let parts: Vec<_> = core.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        bail!("version must look like 0.1.1 or 0.1.1-rc.1");
    }

    for part in parts {
        if !part.chars().all(|c| c.is_ascii_digit()) {
            bail!("version must look like 0.1.1 or 0.1.1-rc.1");
        }
    }

    Ok(())
}

fn validate_prerelease(prerelease: &str) -> Result<()> {
    if prerelease.is_empty() {
        bail!("version must look like 0.1.1 or 0.1.1-rc.1");
    }

    for identifier in prerelease.split('.') {
        if identifier.is_empty() || !is_valid_prerelease_identifier(identifier) {
            bail!("version must look like 0.1.1 or 0.1.1-rc.1");
        }
    }

    Ok(())
}

fn is_valid_prerelease_identifier(identifier: &str) -> bool {
    identifier
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn ensure_git_clean() -> Result<()> {
    let output = Command::new("git")
        .args(["status", "--short"])
        .output()
        .context("failed to run git status --short")?;

    if !output.status.success() {
        bail!("git status --short failed");
    }

    if output.stdout.is_empty() {
        Ok(())
    } else {
        bail!("git working tree is not clean; commit or stash changes before running xtask release")
    }
}

fn ensure_tag_absent(version: &str) -> Result<()> {
    let tag = format!("v{version}");
    let status = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &tag])
        .status()
        .with_context(|| format!("failed to check whether tag {tag} exists"))?;

    if status.success() {
        bail!("git tag {tag} already exists");
    }

    Ok(())
}

fn update_lockfile() -> Result<()> {
    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .status()
        .context("failed to run cargo generate-lockfile")?;

    if status.success() {
        Ok(())
    } else {
        bail!("cargo generate-lockfile failed")
    }
}

fn commit_and_tag(version: &str) -> Result<()> {
    run_git(["add", ROOT_CARGO_TOML, ROOT_CARGO_LOCK])?;
    run_git(["commit", "-m", &format!("Release v{version}")])?;
    run_git(["tag", &format!("v{version}")])?;
    Ok(())
}

fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .context("failed to run git branch --show-current")?;

    if !output.status.success() {
        bail!("git branch --show-current failed");
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        bail!("release helper must be run from a named branch");
    }

    Ok(branch)
}

fn push_release(branch: &str, version: &str) -> Result<()> {
    run_git(["push", "origin", branch])?;
    run_git(["push", "origin", &format!("v{version}")])?;
    Ok(())
}

fn run_git<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if status.success() {
        Ok(())
    } else {
        bail!("git {} failed", args.join(" "))
    }
}

fn restore_release_files(original_manifest: &str) -> Result<()> {
    fs::write(ROOT_CARGO_TOML, original_manifest)
        .with_context(|| format!("failed to restore {ROOT_CARGO_TOML}"))?;
    update_lockfile()?;
    Ok(())
}

fn package_version(contents: &str) -> Result<String> {
    let mut in_package = false;

    for line in contents.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }

        if in_package && trimmed.starts_with("version = ") {
            return parse_version_line(trimmed);
        }
    }

    bail!("failed to find package version in {ROOT_CARGO_TOML}")
}

fn replace_package_version(contents: &str, version: &str) -> Result<String> {
    let mut in_package = false;
    let mut replaced = false;
    let mut output = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
        }

        if in_package && trimmed.starts_with("version = ") && !replaced {
            let indent_len = line.len() - line.trim_start().len();
            let indent = &line[..indent_len];
            output.push(format!("{indent}version = \"{version}\""));
            replaced = true;
            continue;
        }

        output.push(line.to_string());
    }

    if !replaced {
        bail!("failed to update package version in {ROOT_CARGO_TOML}");
    }

    let mut updated = output.join("\n");
    if contents.ends_with('\n') {
        updated.push('\n');
    }
    Ok(updated)
}

fn parse_version_line(line: &str) -> Result<String> {
    let (_, value) = line
        .split_once('=')
        .context("invalid version line in Cargo.toml")?;
    let version = value.trim().trim_matches('"');

    if version.is_empty() {
        bail!("package version in Cargo.toml is empty");
    }

    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::{package_version, replace_package_version, validate_version};

    #[test]
    fn validates_semver_like_version() {
        assert!(validate_version("0.1.1").is_ok());
        assert!(validate_version("0.1.1-rc.1").is_ok());
        assert!(validate_version("0.1.1-beta.2").is_ok());
        assert!(validate_version("1.2.3-alpha-1").is_ok());
        assert!(validate_version("1.0").is_err());
        assert!(validate_version("v1.0.0").is_err());
        assert!(validate_version("1.0.0-").is_err());
        assert!(validate_version("1.0.0-rc..1").is_err());
        assert!(validate_version("1.0.0+build.1").is_err());
    }

    #[test]
    fn reads_root_package_version() {
        let manifest = r#"[package]
name = "jabberwok"
version = "0.1.0"

[workspace]
members = ["xtask"]
"#;

        assert_eq!(package_version(manifest).unwrap(), "0.1.0");
    }

    #[test]
    fn only_rewrites_package_section_version() {
        let manifest = r#"[package]
name = "jabberwok"
version = "0.1.0"

[workspace]
members = ["xtask"]

[patch.crates-io]
example = { version = "1.2.3" }
"#;

        let updated = replace_package_version(manifest, "0.1.1").unwrap();
        assert!(updated.contains("version = \"0.1.1\""));
        assert!(updated.contains("example = { version = \"1.2.3\" }"));
        assert!(!updated.contains("version = \"0.1.0\""));
    }
}
