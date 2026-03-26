use anyhow::Result;

pub fn package() -> Result<()> {
    anyhow::bail!("Windows packaging is not implemented yet")
}

pub fn install_service() -> Result<()> {
    // TODO: create_dir_all(%APPDATA%\jabberwok\logs\) before installing the service
    anyhow::bail!("Windows service installation is not implemented yet")
}

pub fn uninstall_service() -> Result<()> {
    anyhow::bail!("Windows service removal is not implemented yet")
}
