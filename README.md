# Metadata Scrub

A small privacy tool that removes embedded metadata from your photos and videos.

Every photo you take can carry hidden information: GPS coordinates, camera model, timestamps, editing software, and more. Metadata Scrub strips this data out so files are safe to share.

## Features

- Removes EXIF, XMP, and IPTC metadata from images
- Removes QuickTime (`udta`, `meta`, `keys`, `ilst`) and RIFF INFO metadata from videos
- Simple graphical interface, plus a command line mode for scripting
- Scrub a single file, a folder, or a whole folder tree
- Preview changes with a dry run before touching anything
- Optional `.bak` backups, or write cleaned copies to a separate folder and keep the originals untouched
- Files are written atomically, so an interrupted run never leaves a half-written file behind

## Supported formats

| Type | Formats |
|------|---------|
| Images | JPG / JPEG, PNG, WebP |
| Videos | MP4, M4V, MOV, AVI |

Files are detected by their actual content, not just the file extension. Anything else is reported and skipped.

## Installation

Requires [Rust](https://rustup.rs/).

```sh
cargo install --path .
```

On Linux you can also add a launcher icon and desktop entry:

```sh
fish install-desktop.fish
```

## Usage

### Graphical interface

Run the app without arguments to open the GUI:

```sh
metadata_scrub
```

Pick a file or folder, optionally set an output folder, and hit **Scrub**.

### Command line

```sh
# Scrub a single file in place
metadata_scrub photo.jpg

# Scrub everything in a folder (not subfolders)
metadata_scrub ~/Pictures/holiday

# Scrub a folder and all subfolders
metadata_scrub -r ~/Pictures

# Preview what would be removed, without modifying anything
metadata_scrub -n -r ~/Pictures

# Keep a .bak copy next to each scrubbed file
metadata_scrub -b photo.jpg

# Write cleaned copies to another folder, originals stay untouched
metadata_scrub -o ~/clean ~/Pictures/holiday
```

### Options

| Flag | Description |
|------|-------------|
| `--gui` | Launch the graphical interface |
| `-r`, `--recursive` | Recurse into subdirectories |
| `-n`, `--dry-run` | Show what would be done without modifying any files |
| `-b`, `--backup` | Save a `<file>.bak` copy next to each scrubbed file |
| `-o`, `--output <DIR>` | Write cleaned copies into `DIR` instead of scrubbing in place |

## What gets removed

- **JPEG**: EXIF (including GPS data) and XMP blocks, plus metadata comments. Color profiles (ICC) are kept.
- **PNG**: `eXIf` chunk and XMP / EXIF / IPTC / Photoshop text chunks. Other text chunks are kept.
- **WebP**: EXIF and XMP chunks.
- **MP4 / M4V / MOV**: QuickTime user data (`udta`), `meta`, `keys`, `ilst` boxes and XMP. Video and audio tracks are untouched.
- **AVI**: INFO lists and EXIF/XMP chunks.

Scrubbing never re-encodes your files; only metadata is removed, so image and video quality are unchanged.

## Tips

- If you are not sure what will be removed, run with `-n` (dry run) first.
- By default files are modified **in place**. Use `-o DIR` or `-b` if you want to keep the original data.
- Metadata can also hide in files that *look* clean. Scrubbing is idempotent: running it again on a clean file is harmless.

## License

This is free and unencumbered software released into the public domain — [The Unlicense](https://unlicense.org/).
