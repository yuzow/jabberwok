use anyhow::Result;

pub fn package() -> Result<()> {
    anyhow::bail!("Linux packaging is not implemented yet")
}

pub fn install_service() -> Result<()> {
    // TODO: create_dir_all($XDG_STATE_HOME/jabberwok/logs/ or ~/.local/state/jabberwok/logs/) before installing the service
    anyhow::bail!("Linux service installation is not implemented yet")
}

pub fn uninstall_service() -> Result<()> {
    anyhow::bail!("Linux service removal is not implemented yet")
}
