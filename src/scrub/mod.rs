pub mod jpeg;
pub mod mp4;
pub mod png;
pub mod riff;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub dry_run: bool,
    pub backup: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Scrubbed { before: u64, after: u64, dry_run: bool },
    Clean { copied_to: Option<PathBuf> },
    Unsupported { reason: String },
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Scrubbed { before, after, dry_run } => {
                let saved = before.saturating_sub(*after);
                if *dry_run {
                    format!("would remove {saved} bytes of metadata (dry run)")
                } else {
                    format!("removed {saved} bytes of metadata ({before} -> {after} bytes)")
                }
            }
            Outcome::Clean { copied_to: Some(dest) } => {
                format!("no metadata found, copied to {}", dest.display())
            }
            Outcome::Clean { copied_to: None } => "no removable metadata found".to_string(),
            Outcome::Unsupported { reason } => format!("unsupported: {reason}"),
        }
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Outcome::Unsupported { .. })
    }
}

#[derive(Debug)]
pub struct Stats {
    pub scrubbed: usize,
    pub clean: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub stopped: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Job {
    pub recursive: bool,
    pub dry_run: bool,
    pub backup: bool,
    pub output_dir: Option<PathBuf>,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "mp4", "m4v", "mov", "avi"];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn collect_files(root: &Path, recursive: bool) -> anyhow::Result<Vec<PathBuf>> {
    if root.is_file() {
        if is_supported(root) {
            return Ok(vec![root.to_path_buf()]);
        }
        let ext = root
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_else(|| "<none>".to_string());
        bail!("unsupported file type {ext} (supported: {})", SUPPORTED_EXTENSIONS.join(", "));
    }

    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false);
    let walker = if recursive { walker } else { walker.max_depth(1) };
    for entry in walker {
        let entry = entry.with_context(|| format!("failed to read {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if is_supported(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Debug)]
enum Kind {
    Jpeg,
    Png,
    Webp,
    Avi,
    Mp4,
    Unknown(String),
}

fn detect(data: &[u8], path: &Path) -> Kind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Kind::Jpeg;
    }
    if data.len() >= 8 && data[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Kind::Png;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" {
        match &data[8..12] {
            b"WEBP" => return Kind::Webp,
            b"AVI " => return Kind::Avi,
            _ => {}
        }
        return Kind::Unknown(format!("unrecognized RIFF container (.{ext})"));
    }
    if data.len() >= 12 && (&data[4..8] == b"ftyp" || &data[4..8] == b"moov" || &data[4..8] == b"wide") {
        return Kind::Mp4;
    }
    Kind::Unknown(format!("unrecognized format (.{ext})"))
}

pub fn process_file(path: &Path, dest: Option<&Path>, opts: &Options) -> anyhow::Result<Outcome> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;

    let cleaned = match detect(&data, path) {
        Kind::Jpeg => jpeg::scrub(&data)?,
        Kind::Png => png::scrub(&data)?,
        Kind::Webp => riff::scrub_webp(&data)?,
        Kind::Avi => riff::scrub_avi(&data)?,
        Kind::Mp4 => mp4::scrub(&data)?,
        Kind::Unknown(reason) => return Ok(Outcome::Unsupported { reason }),
    };

    let Some(cleaned) = cleaned else {
        return match dest {
            Some(dest) => {
                copy_unchanged(path, dest)?;
                Ok(Outcome::Clean { copied_to: Some(dest.to_path_buf()) })
            }
            None => Ok(Outcome::Clean { copied_to: None }),
        };
    };

    match dest {
        Some(dest) => atomic_write(dest, &cleaned)
            .with_context(|| format!("failed to write {}", dest.display()))?,
        None => {
            if !opts.dry_run {
                if opts.backup {
                    let mut backup_path = path.as_os_str().to_os_string();
                    backup_path.push(".bak");
                    fs::copy(path, &backup_path).with_context(|| {
                        format!("failed to create backup {}", Path::new(&backup_path).display())
                    })?;
                }
                atomic_write(path, &cleaned)
                    .with_context(|| format!("failed to write {}", path.display()))?;
            }
        }
    }

    Ok(Outcome::Scrubbed {
        before: data.len() as u64,
        after: cleaned.len() as u64,
        dry_run: opts.dry_run && dest.is_none(),
    })
}

pub fn run_job(
    root: &Path,
    job: &Job,
    stop: Option<&std::sync::atomic::AtomicBool>,
    mut on_file: impl FnMut(&Path, anyhow::Result<Outcome>),
) -> anyhow::Result<Stats> {
    use std::sync::atomic::Ordering;

    let files = collect_files(root, job.recursive)?;
    let mut stats = Stats { scrubbed: 0, clean: 0, unsupported: 0, failed: 0, stopped: false };

    for path in files {
        if let Some(stop) = stop
            && stop.load(Ordering::Relaxed)
        {
            stats.stopped = true;
            return Ok(stats);
        }

        let dest = job.output_dir.as_ref().map(|out| {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.join(rel)
        });
        let opts = Options { dry_run: job.dry_run, backup: job.backup };

        let result = process_file(&path, dest.as_deref(), &opts);
        let unsupported = matches!(&result, Ok(outcome) if outcome.is_unsupported());
        if result.is_err() {
            stats.failed += 1;
        } else if unsupported {
            stats.unsupported += 1;
        } else {
            match &result {
                Ok(Outcome::Scrubbed { .. }) => stats.scrubbed += 1,
                Ok(Outcome::Clean { .. }) => stats.clean += 1,
                _ => unreachable!(),
            }
        }
        on_file(&path, result);
    }
    Ok(stats)
}

fn copy_unchanged(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if fs::copy(src, dest).is_err() {
        atomic_write(dest, &fs::read(src)?)?;
    }
    Ok(())
}

fn atomic_write(dest: &Path, data: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = dest.with_file_name(format!(
        ".{}.scrub-tmp-{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, dest)
        .with_context(|| format!("failed to replace {}", dest.display()))?;
    Ok(())
}
