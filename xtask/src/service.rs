use anyhow::Result;

use crate::Platform;

pub fn install_service(platform: Platform) -> Result<()> {
    match platform {
        Platform::Macos => crate::macos::install_service(),
        Platform::Windows => crate::windows::install_service(),
        Platform::Linux => crate::linux::install_service(),
    }
}

pub fn uninstall_service(platform: Platform) -> Result<()> {
    match platform {
        Platform::Macos => crate::macos::uninstall_service(),
        Platform::Windows => crate::windows::uninstall_service(),
        Platform::Linux => crate::linux::uninstall_service(),
    }
}
