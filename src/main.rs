use clap::Parser;

fn main() {
    if let Err(error) = swmctl::cli::run(swmctl::cli::Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
