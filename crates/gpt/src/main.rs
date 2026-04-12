use clap::Parser;

fn main() {
    let cli = gpt::Cli::parse();
    match gpt::run(&cli) {
        Ok(output) => println!("{output}"),
        Err(err) => {
            eprintln!("{err:#}");
            std::process::exit(1);
        }
    }
}
