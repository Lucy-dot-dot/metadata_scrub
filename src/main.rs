use clap::Parser as _;

mod cli;
mod gui;
mod scrub;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    match args.path {
        Some(_) if !args.gui => cli::run(args),
        _ => gui::run(),
    }
}
