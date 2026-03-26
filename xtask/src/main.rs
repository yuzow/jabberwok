mod linux;
mod macos;
mod package;
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
    InstallService {
        #[arg(value_enum)]
        platform: Platform,
    },
    UninstallService {
        #[arg(value_enum)]
        platform: Platform,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Package { platform } => package::package(platform),
        Command::InstallService { platform } => service::install_service(platform),
        Command::UninstallService { platform } => service::uninstall_service(platform),
    }
}
