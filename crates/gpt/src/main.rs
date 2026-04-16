use clap::Parser;

fn main() {
    let cli = gpt::Cli::parse();
    match gpt::run(&cli) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
        }
        Err(err) => {
            eprintln!("{}", gpt::format_cli_error(&err));
            std::process::exit(1);
        }
    }
}
