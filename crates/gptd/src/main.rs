use clap::Parser;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gptd", version, about = "Tablinq daemon scaffold")]
struct Cli {
    #[arg(long, default_value = ".tablinq/state")]
    state_dir: PathBuf,
    #[arg(long, default_value_t = false)]
    warm: bool,
}

fn main() {
    let cli = Cli::parse();
    let payload = json!({
        "status": "scaffold",
        "protocol_version": shared_protocol::PROTOCOL_VERSION,
        "state_dir": cli.state_dir,
        "warm": cli.warm,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("serialize daemon scaffold")
    );
}
