use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gptd", version, about = "Tablinq local daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Version {
        #[arg(long)]
        json: bool,
    },
    Serve {
        #[arg(long)]
        state_root: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Version { json } => {
            println!("{}", gptd::build_info::render(json));
            Ok(())
        }
        Command::Serve { state_root } => {
            let config = gptd::AppConfig::load_or_create(state_root.as_deref())?;
            gptd::run_service(config)
        }
    }
}
