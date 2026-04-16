mod linux;
mod macos;
mod package;
mod release;
mod service;
mod windows;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Build and packaging helpers for jabberwok")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Package {
        #[arg(value_enum)]
        platform: Platform,
    },
    PackageMacos {
        #[arg(value_enum, default_value_t = MacosPackageStage::All)]
        stage: MacosPackageStage,
    },
    InstallService {
        #[arg(value_enum)]
        platform: Platform,
    },
    UninstallService {
        #[arg(value_enum)]
        platform: Platform,
    },
    Release {
        version: String,
        #[arg(long)]
        push: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum MacosPackageStage {
    BuildBinary,
    StageApp,
    PackageDmg,
    All,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Package { platform } => package::package(platform),
        Command::PackageMacos { stage } => package::package_macos(stage),
        Command::InstallService { platform } => service::install_service(platform),
        Command::UninstallService { platform } => service::uninstall_service(platform),
        Command::Release { version, push } => release::prepare_release(&version, push),
    }
}
