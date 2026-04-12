use clap::Parser;

fn main() {
    let cli = gpt::Cli::parse();
    println!("{}", gpt::render_command_summary(&cli));
}
