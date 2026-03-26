use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};

use super::schema::{DevicePrefs, DevicesConfig, JabberwokConfig};

pub fn current_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

pub fn device_prefs_for_current_host(config: &DevicesConfig) -> &DevicePrefs {
    let host = current_hostname();
    config
        .hosts
        .get(&host)
        .or_else(|| config.hosts.get("default"))
        .unwrap_or(&DevicePrefs::DEFAULT)
}

pub fn resolve_input_device(
    hostname: &str,
    prefs: &mut DevicePrefs,
    config_path: &Path,
) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match prefs.input.clone() {
        Some(name) => {
            if let Some(device) = host.input_devices()?.find(|device| {
                device.description().ok().map(|desc| desc.name().to_owned()) == Some(name.clone())
            }) {
                return Ok(device);
            }

            prefs.input = None;
            persist_cleared_device_preference(hostname, prefs, config_path, true, &name);
            let default = host
                .default_input_device()
                .ok_or_else(|| anyhow::anyhow!("no default input device"))?;
            let fallback = default
                .description()
                .ok()
                .map(|desc| desc.name().to_owned())
                .unwrap_or_else(|| "<unknown>".to_string());
            tracing::warn!(
                host = hostname,
                device_kind = "input",
                configured_device = %name,
                fallback_device = %fallback,
                "configured device unavailable; falling back to OS default"
            );
            Ok(default)
        }
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default input device")),
    }
}

#[allow(dead_code)]
pub fn resolve_output_device(
    hostname: &str,
    prefs: &mut DevicePrefs,
    config_path: &Path,
) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match prefs.output.clone() {
        Some(name) => {
            if let Some(device) = host.output_devices()?.find(|device| {
                device.description().ok().map(|desc| desc.name().to_owned()) == Some(name.clone())
            }) {
                return Ok(device);
            }

            prefs.output = None;
            persist_cleared_device_preference(hostname, prefs, config_path, false, &name);
            let default = host
                .default_output_device()
                .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
            let fallback = default
                .description()
                .ok()
                .map(|desc| desc.name().to_owned())
                .unwrap_or_else(|| "<unknown>".to_string());
            tracing::warn!(
                host = hostname,
                device_kind = "output",
                configured_device = %name,
                fallback_device = %fallback,
                "configured device unavailable; falling back to OS default"
            );
            Ok(default)
        }
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no default output device")),
    }
}

pub fn update_device_preference(
    config_path: &Path,
    prefs_store: Option<&Arc<Mutex<DevicePrefs>>>,
    is_input: bool,
    selected: Option<String>,
    source: &str,
) -> Result<()> {
    let host = current_hostname();
    let mut config = JabberwokConfig::load(config_path)?;

    let entry = config.devices.hosts.entry(host.clone()).or_default();
    if is_input {
        entry.input = selected.clone();
    } else {
        entry.output = selected.clone();
    }

    config.save(config_path)?;

    tracing::info!(
        source,
        host = host,
        device_kind = if is_input { "input" } else { "output" },
        selected = selected.as_deref().unwrap_or("OS default"),
        "device preference updated"
    );

    if let Some(prefs_store) = prefs_store {
        match prefs_store.lock() {
            Ok(mut guard) => {
                if is_input {
                    guard.input = selected;
                } else {
                    guard.output = selected;
                }
            }
            Err(_) => {
                tracing::warn!(
                    "device preference lock is poisoned; cached preference update may be stale"
                );
            }
        }
    }

    Ok(())
}

fn persist_cleared_device_preference(
    hostname: &str,
    prefs: &DevicePrefs,
    config_path: &Path,
    is_input: bool,
    missing: &str,
) {
    let mut config = match JabberwokConfig::load(config_path) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(
                host = hostname,
                device_kind = if is_input { "input" } else { "output" },
                configured_device = %missing,
                error = %error,
                "failed to load config while clearing stale device preference"
            );
            return;
        }
    };

    config
        .devices
        .hosts
        .insert(hostname.to_string(), prefs.clone());

    if let Err(error) = config.save(config_path) {
        tracing::error!(
            host = hostname,
            device_kind = if is_input { "input" } else { "output" },
            configured_device = %missing,
            error = %error,
            "failed to persist cleared stale device preference"
        );
        return;
    }

    tracing::info!(
        source = "auto_fallback",
        host = hostname,
        device_kind = if is_input { "input" } else { "output" },
        configured_device = %missing,
        selected = "OS default",
        "device preference updated"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_cleared_device_preference_writes_current_host_effective_prefs() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");

        let mut config = JabberwokConfig::default();
        config.devices.hosts.insert(
            "default".to_string(),
            DevicePrefs {
                input: Some("USB Mic".to_string()),
                output: Some("Bluetooth Headphones".to_string()),
            },
        );
        config.save(&config_path).unwrap();

        let effective = DevicePrefs {
            input: Some("USB Mic".to_string()),
            output: None,
        };
        persist_cleared_device_preference(
            "work-mac",
            &effective,
            &config_path,
            false,
            "Bluetooth Headphones",
        );

        let updated = JabberwokConfig::load(&config_path).unwrap();
        let host = updated.devices.hosts.get("work-mac").unwrap();
        assert_eq!(host.input.as_deref(), Some("USB Mic"));
        assert_eq!(host.output, None);
        let default = updated.devices.hosts.get("default").unwrap();
        assert_eq!(default.input.as_deref(), Some("USB Mic"));
        assert_eq!(default.output.as_deref(), Some("Bluetooth Headphones"));
    }
}
