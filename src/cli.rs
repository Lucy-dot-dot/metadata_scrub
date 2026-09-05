use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;

use crate::scrub::{self, Job, Outcome};

#[derive(Parser, Debug)]
#[command(
    name = "metadata_scrub",
    version,
    about = "Remove EXIF and other metadata from photos and videos",
    long_about = "Remove EXIF and other metadata (XMP, IPTC, QuickTime udta) from photos and videos.\n\
        Pass a file or a directory to scrub in place, or no arguments to launch the GUI."
)]
pub struct Args {
    /// Photo/video file or directory to scrub. Omit to launch the GUI.
    pub path: Option<PathBuf>,

    /// Launch the graphical interface
    #[arg(long)]
    pub gui: bool,

    /// Recurse into subdirectories
    #[arg(short, long)]
    pub recursive: bool,

    /// Show what would be done without modifying any files
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Save a <file>.bak copy next to each scrubbed file
    #[arg(short, long)]
    pub backup: bool,

    /// Write cleaned copies into DIR (preserving relative structure) instead of scrubbing in place
    #[arg(short, long, value_name = "DIR", conflicts_with = "backup")]
    pub output: Option<PathBuf>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    if args.path.is_none() {
        bail!("a PATH is required unless the GUI is used");
    }

    let job = Job {
        recursive: args.recursive,
        dry_run: args.dry_run,
        backup: args.backup,
        output_dir: args.output.clone(),
    };
    let root = args.path.unwrap();
    if let Some(out) = &job.output_dir
        && !out.exists()
    {
        std::fs::create_dir_all(out)?;
    }

    let mut failures: Vec<String> = Vec::new();
    let stats = scrub::run_job(&root, &job, None, |path, result| match result {
        Ok(outcome @ Outcome::Scrubbed { .. }) => {
            println!("scrubbed  {}  ({})", path.display(), outcome.label());
        }
        Ok(outcome @ Outcome::Clean { .. }) => {
            println!("clean     {}  ({})", path.display(), outcome.label());
        }
        Ok(outcome @ Outcome::Unsupported { .. }) => {
            println!("skipped   {}  ({})", path.display(), outcome.label());
        }
        Err(err) => {
            println!("failed    {}  ({err:#})", path.display());
            failures.push(format!("{}: {err:#}", path.display()));
        }
    })?;

    if stats.stopped {
        bail!("interrupted");
    }

    println!(
        "\n{} scrubbed, {} already clean, {} unsupported, {} failed",
        stats.scrubbed, stats.clean, stats.unsupported, stats.failed
    );
    if !failures.is_empty() {
        bail!("{} file(s) failed to process", failures.len());
    }
    if args.dry_run {
        println!("dry run: no files were modified");
    }

    Ok(())
}
