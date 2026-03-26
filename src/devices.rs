use crate::config::JabberwokConfig;
use anyhow::{Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use std::path::Path;

use crate::config::DevicePrefs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInventory {
    pub default_input_name: Option<String>,
    pub default_output_name: Option<String>,
    pub input_names: Vec<String>,
    pub output_names: Vec<String>,
}

pub fn inventory() -> Result<DeviceInventory> {
    let host = cpal::default_host();
    let default_output_name = host
        .default_output_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_owned()));
    let default_input_name = host
        .default_input_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_owned()));

    let mut output_names = Vec::new();
    for device in host.output_devices()? {
        output_names.push(device.description()?.name().to_owned());
    }

    let mut input_names = Vec::new();
    for device in host.input_devices()? {
        input_names.push(device.description()?.name().to_owned());
    }

    Ok(DeviceInventory {
        default_input_name,
        default_output_name,
        input_names,
        output_names,
    })
}

pub fn log_inventory() -> Result<()> {
    let inventory = inventory()?;
    tracing::info!(devices = ?inventory.input_names, "audio input");
    tracing::info!(selected_input = inventory.default_input_name.as_deref().unwrap_or("<none>"));
    tracing::info!(devices = ?inventory.output_names, "audio output");
    tracing::info!(selected_output = inventory.default_output_name.as_deref().unwrap_or("<none>"));
    Ok(())
}

pub fn list_devices(prefs: &DevicePrefs, hostname: &str, from_fallback: bool) -> Result<()> {
    let inventory = inventory()?;
    let lines = list_device_lines(&inventory, prefs, hostname, from_fallback);
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

pub fn select_device(
    input_selector: Option<&str>,
    output_selector: Option<&str>,
    target_host: &str,
    config_path: &Path,
) -> Result<(Option<String>, Option<String>)> {
    if input_selector.is_none() && output_selector.is_none() {
        bail!("select-device requires --input and/or --output");
    }

    let inventory = inventory()?;
    let resolved_input = match input_selector {
        Some(selector) => resolve_device_selector(&inventory.input_names, selector)?,
        None => None,
    };
    let resolved_output = match output_selector {
        Some(selector) => resolve_device_selector(&inventory.output_names, selector)?,
        None => None,
    };

    let mut config = JabberwokConfig::load(config_path)?;
    let prefs = config
        .devices
        .hosts
        .entry(target_host.to_string())
        .or_default();
    if input_selector.is_some() {
        prefs.input = resolved_input.map(|name| name.to_string());
    }
    if output_selector.is_some() {
        prefs.output = resolved_output.map(|name| name.to_string());
    }
    let result = (
        resolved_input.map(str::to_string),
        resolved_output.map(str::to_string),
    );
    config.save(config_path)?;

    if input_selector.is_some() {
        tracing::info!(
            source = "cli",
            host = target_host,
            device_kind = "input",
            selected = result.0.as_deref().unwrap_or("OS default"),
            "device preference updated"
        );
    }
    if output_selector.is_some() {
        tracing::info!(
            source = "cli",
            host = target_host,
            device_kind = "output",
            selected = result.1.as_deref().unwrap_or("OS default"),
            "device preference updated"
        );
    }

    Ok(result)
}

fn list_device_lines(
    inventory: &DeviceInventory,
    prefs: &DevicePrefs,
    hostname: &str,
    from_fallback: bool,
) -> Vec<String> {
    let header = if from_fallback {
        format!(r#"Devices for host: {hostname} (using "default" config)"#)
    } else {
        format!("Devices for host: {hostname}")
    };

    let mut lines = Vec::new();
    lines.push(header);
    lines.push(String::new());
    lines.push("Output devices:".to_string());
    lines.extend(format_device_lines(
        &inventory.output_names,
        inventory.default_output_name.as_deref(),
        prefs.output.as_deref(),
    ));
    lines.push(String::new());
    lines.push("Input devices:".to_string());
    lines.extend(format_device_lines(
        &inventory.input_names,
        inventory.default_input_name.as_deref(),
        prefs.input.as_deref(),
    ));
    lines.push(String::new());
    lines.push("* = currently active or selected".to_string());
    lines
}

fn format_device_lines(
    names: &[String],
    default_name: Option<&str>,
    selected_name: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let selected = selected_name.is_some_and(|selected| selected == name.as_str());
        let defaulted = default_name.is_some_and(|default| default == name.as_str());

        let mut suffixes = Vec::new();
        if defaulted {
            suffixes.push("(default)");
        }
        let marker = if selected || (selected_name.is_none() && defaulted) {
            "*"
        } else {
            " "
        };

        if suffixes.is_empty() {
            out.push(format!("{:>2}{marker}  {name}", i + 1));
        } else {
            out.push(format!(
                "{:>2}{marker}  {name} {}",
                i + 1,
                suffixes.join(" ")
            ));
        }
    }
    out
}

pub fn resolve_device_selector<'a>(
    devices: &'a [String],
    selector: &str,
) -> Result<Option<&'a str>> {
    if selector.is_empty() || selector == "0" {
        return Ok(None);
    }

    if let Ok(index) = selector.parse::<usize>() {
        if index == 0 {
            return Ok(None);
        }
        if let Some(name) = devices.get(index - 1) {
            return Ok(Some(name.as_str()));
        }
        bail!(
            "device index {index} is out of range; expected 1..={}",
            devices.len()
        );
    }

    let exact_matches: Vec<&String> = devices
        .iter()
        .filter(|name| name.eq_ignore_ascii_case(selector))
        .collect();
    if exact_matches.len() == 1 {
        return Ok(Some(exact_matches[0].as_str()));
    }
    if exact_matches.len() > 1 {
        let listed: Vec<_> = exact_matches.iter().map(|s| s.as_str()).collect();
        bail!(
            "ambiguous selector {selector:?}; matches: {}",
            listed.join(", ")
        );
    }

    let selector_lower = selector.to_ascii_lowercase();
    let partial_matches: Vec<&String> = devices
        .iter()
        .filter(|name| name.to_ascii_lowercase().contains(&selector_lower))
        .collect();

    if partial_matches.len() == 1 {
        return Ok(Some(partial_matches[0].as_str()));
    }
    if partial_matches.is_empty() {
        bail!(
            "no device matched {selector:?}; available: {}",
            devices.join(", ")
        );
    }
    bail!(
        "ambiguous selector {selector:?}; matches: {}",
        partial_matches
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_device_selector_index_in_range() {
        let names = vec!["USB".to_string(), "Mic".to_string()];
        let resolved = resolve_device_selector(&names, "2").unwrap();
        assert_eq!(resolved, Some("Mic"));
    }

    #[test]
    fn resolve_device_selector_index_out_of_range_is_error() {
        let names = vec!["USB".to_string(), "Mic".to_string()];
        let err = resolve_device_selector(&names, "3").unwrap_err();
        assert!(err.to_string().contains("out of range"), "error was: {err}");
    }

    #[test]
    fn resolve_device_selector_zero_clears() {
        let names = vec!["USB".to_string()];
        assert_eq!(resolve_device_selector(&names, "0").unwrap(), None);
        assert_eq!(resolve_device_selector(&names, "").unwrap(), None);
    }

    #[test]
    fn resolve_device_selector_exact_name_is_case_insensitive() {
        let names = vec!["My USB Audio".to_string(), "Built-in Mic".to_string()];
        assert_eq!(
            resolve_device_selector(&names, "my usb audio").unwrap(),
            Some("My USB Audio")
        );
    }

    #[test]
    fn resolve_device_selector_partial_name_unambiguous() {
        let names = vec!["My USB Audio".to_string(), "Built-in Mic".to_string()];
        assert_eq!(
            resolve_device_selector(&names, "usb").unwrap(),
            Some("My USB Audio")
        );
    }

    #[test]
    fn resolve_device_selector_ambiguous_partial_match_is_error() {
        let names = vec!["USB Mic".to_string(), "USB Speaker".to_string()];
        let err = resolve_device_selector(&names, "usb").unwrap_err();
        assert!(
            err.to_string().contains("ambiguous selector"),
            "error was: {err}"
        );
    }

    #[test]
    fn resolve_device_selector_no_match_is_error() {
        let names = vec!["USB Mic".to_string(), "Speaker".to_string()];
        let err = resolve_device_selector(&names, "does-not-exist").unwrap_err();
        assert!(
            err.to_string().contains("no device matched"),
            "error was: {err}"
        );
    }

    #[test]
    fn select_device_clears_output_pref_for_target_host_and_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        let mut config = crate::config::JabberwokConfig::default();
        config.devices.hosts.insert(
            "other".to_string(),
            crate::config::DevicePrefs {
                input: Some("Other Input".to_string()),
                output: Some("Other Output".to_string()),
            },
        );
        config.save(&config_path).unwrap();

        let _ = select_device(None, Some("0"), "localhost", &config_path).unwrap();
        let updated = crate::config::JabberwokConfig::load(&config_path).unwrap();

        assert!(updated.devices.hosts.contains_key("other"));
        let other = updated.devices.hosts.get("other").unwrap();
        assert_eq!(other.input.as_deref(), Some("Other Input"));
        assert_eq!(other.output.as_deref(), Some("Other Output"));

        let localhost = updated.devices.hosts.get("localhost").unwrap();
        assert_eq!(localhost.input, None);
        assert_eq!(localhost.output, None);
    }

    #[test]
    fn select_device_preserves_input_pref_when_only_output_is_changed() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        let mut config = crate::config::JabberwokConfig::default();
        config.devices.hosts.insert(
            "localhost".to_string(),
            crate::config::DevicePrefs {
                input: Some("Existing Mic".to_string()),
                output: Some("Existing Speaker".to_string()),
            },
        );
        config.save(&config_path).unwrap();

        let (input_sel, output_sel) =
            select_device(None, Some("0"), "localhost", &config_path).unwrap();

        let updated = crate::config::JabberwokConfig::load(&config_path).unwrap();
        let localhost = updated.devices.hosts.get("localhost").unwrap();
        assert_eq!(localhost.input.as_deref(), Some("Existing Mic"));
        assert_eq!(localhost.output.as_deref(), None);
        assert_eq!(input_sel, None);
        assert_eq!(output_sel, None);
    }

    #[test]
    fn select_device_creates_missing_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("missing.toml");
        let _ = select_device(Some("0"), None, "default", &config_path).unwrap();
        assert!(config_path.exists());
        let loaded = crate::config::JabberwokConfig::load(&config_path).unwrap();
        assert!(loaded.devices.hosts.contains_key("default"));
    }

    #[test]
    fn list_devices_includes_indices_selected_and_default() {
        let inventory = DeviceInventory {
            default_input_name: Some("Mic".to_string()),
            default_output_name: Some("Built-in".to_string()),
            input_names: vec!["Mic".to_string(), "USB Mic".to_string()],
            output_names: vec!["Built-in".to_string()],
        };
        let prefs = DevicePrefs {
            input: Some("USB Mic".to_string()),
            output: Some("Built-in".to_string()),
        };
        let lines = super::list_device_lines(&inventory, &prefs, "macbook-pro", false);

        assert_eq!(lines[0], "Devices for host: macbook-pro");
        assert_eq!(lines[2], "Output devices:");
        assert_eq!(lines[3], " 1*  Built-in (default)");
        assert_eq!(lines[5], "Input devices:");
        assert_eq!(lines[6], " 1   Mic (default)");
        assert_eq!(lines[7], " 2*  USB Mic");
        assert_eq!(lines[9], "* = currently active or selected");
    }

    #[test]
    fn list_devices_marks_default_when_no_selection_is_set() {
        let inventory = DeviceInventory {
            default_input_name: Some("Mic".to_string()),
            default_output_name: Some("Built-in".to_string()),
            input_names: vec!["Mic".to_string(), "USB Mic".to_string()],
            output_names: vec!["Built-in".to_string()],
        };
        let prefs = DevicePrefs::default();
        let lines = super::list_device_lines(&inventory, &prefs, "mba", false);

        assert_eq!(lines[3], " 1*  Built-in (default)");
        assert_eq!(lines[5], "Input devices:");
        assert_eq!(lines[6], " 1*  Mic (default)");
        assert_eq!(lines[7], " 2   USB Mic");
        assert_eq!(lines[9], "* = currently active or selected");
    }

    #[test]
    fn list_devices_includes_fallback_header() {
        let inventory = DeviceInventory {
            default_input_name: None,
            default_output_name: None,
            input_names: vec!["Mic".to_string()],
            output_names: vec!["Spk".to_string()],
        };
        let prefs = DevicePrefs::default();
        let lines = super::list_device_lines(&inventory, &prefs, "studio-mac", true);
        assert_eq!(
            lines[0],
            r#"Devices for host: studio-mac (using "default" config)"#
        );
    }

    #[test]
    fn list_devices_succeeds() {
        assert!(list_devices(&DevicePrefs::default(), "localhost", false).is_ok());
    }
}
