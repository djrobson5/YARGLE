//! Detect and unpack downloaded archives (zip, 7z, RAR4/RAR5).
//!
//! All three formats share one layout rule: an archive whose entries sit
//! under a single top-level item (the usual `Artist - Song/` folder, or a
//! lone CON file) extracts straight into the library; anything else gets a
//! folder named after the archive, so loose files never spill into the
//! library root.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Zip,
    SevenZ,
    Rar,
}

/// Identify an archive by its magic bytes. `None` = not an archive (a raw
/// CON/STFS package, a `.sng`, …), which the caller saves as-is.
pub fn detect(head: &[u8]) -> Option<Kind> {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Some(Kind::Zip)
    } else if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(Kind::SevenZ)
    } else if head.starts_with(b"Rar!\x1A\x07") {
        // 1A 07 00 = RAR 1.5–4.x, 1A 07 01 00 = RAR5; unrar handles both.
        Some(Kind::Rar)
    } else {
        None
    }
}

/// Extract `archive` into `dest`. `name_hint` (usually the download's file
/// name) names the wrapper folder when the archive has no single top-level
/// item. Returns the extracted song's path and the number of files written.
/// On failure, anything this call created is removed again.
pub fn extract(kind: Kind, archive: &Path, dest: &Path, name_hint: &str) -> Result<(PathBuf, usize), String> {
    let entries = list(kind, archive)?;
    let mut tops: Vec<&str> = Vec::new();
    let mut files = 0usize;
    for (name, is_dir) in &entries {
        if !is_safe(name) {
            return Err("This archive contains unsafe file paths, so YARGLE won't extract it.".into());
        }
        if !is_dir {
            files += 1;
        }
        if let Some(first) = name.split('/').find(|s| !s.is_empty()) {
            if !tops.contains(&first) {
                tops.push(first);
            }
        }
    }
    if files == 0 {
        return Err("The archive is empty.".into());
    }

    let (base, top) = if tops.len() == 1 {
        (dest.to_path_buf(), dest.join(tops[0]))
    } else {
        let folder = dest.join(wrapper_name(name_hint));
        (folder.clone(), folder)
    };
    let created = !top.exists();

    let result = fs::create_dir_all(&base)
        .map_err(|e| format!("Failed to create folder: {}", e))
        .and_then(|_| match kind {
            Kind::Zip => extract_zip(archive, &base),
            Kind::SevenZ => extract_7z(archive, &base),
            Kind::Rar => extract_rar(archive, &base),
        });
    if let Err(e) = result {
        if created {
            let _ = if top.is_dir() { fs::remove_dir_all(&top) } else { fs::remove_file(&top) };
        }
        return Err(e);
    }
    Ok((top, files))
}

/// macOS resource-fork junk that some zips carry; never extracted.
fn is_junk(name: &str) -> bool {
    name.split('/').any(|s| s == "__MACOSX" || s == ".DS_Store")
}

/// Relative, with no `..`, root or drive components.
fn is_safe(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Entry names, `/`-separated, junk dropped, with an is-directory flag.
fn list(kind: Kind, archive: &Path) -> Result<Vec<(String, bool)>, String> {
    let raw: Vec<(String, bool)> = match kind {
        Kind::Zip => {
            let mut zip = zip::ZipArchive::new(open(archive)?).map_err(zip_error)?;
            (0..zip.len())
                .map(|i| {
                    let f = zip.by_index_raw(i).map_err(zip_error)?;
                    Ok((f.name().to_string(), f.is_dir()))
                })
                .collect::<Result<_, String>>()?
        }
        Kind::SevenZ => {
            let reader = sevenz_rust2::ArchiveReader::new(open(archive)?, sevenz_rust2::Password::empty())
                .map_err(sevenz_error)?;
            reader
                .archive()
                .files
                .iter()
                .map(|f| (f.name().to_string(), f.is_directory()))
                .collect()
        }
        Kind::Rar => {
            let listing = unrar_ng::Archive::new(archive).open_for_listing().map_err(rar_error)?;
            let mut out = Vec::new();
            for entry in listing {
                let e = entry.map_err(rar_error)?;
                if e.is_split() {
                    return Err("This RAR is split into several parts, which YARGLE can't join. \
                                Use Open ↗ to download all the parts manually."
                        .into());
                }
                out.push((e.filename.to_string_lossy().into_owned(), e.is_directory()));
            }
            out
        }
    };
    Ok(raw
        .into_iter()
        .map(|(n, d)| (n.replace('\\', "/").trim_start_matches("./").to_string(), d))
        .filter(|(n, _)| !n.is_empty() && !is_junk(n))
        .collect())
}

fn extract_zip(archive: &Path, base: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(open(archive)?).map_err(zip_error)?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(zip_error)?;
        // enclosed_name() returns None for unsafe (path-traversal) entries.
        let Some(rel) = file.enclosed_name() else { continue };
        if is_junk(&rel.to_string_lossy().replace('\\', "/")) {
            continue;
        }
        let outpath = base.join(&rel);
        if file.is_dir() {
            fs::create_dir_all(&outpath).map_err(|e| format!("mkdir failed: {}", e))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
            }
            let mut out = File::create(&outpath).map_err(|e| format!("write failed: {}", e))?;
            io::copy(&mut file, &mut out).map_err(|e| {
                if e.to_string().to_ascii_lowercase().contains("checksum") {
                    DAMAGED.to_string()
                } else {
                    format!("Extraction failed: {}", e)
                }
            })?;
        }
    }
    Ok(())
}

fn extract_7z(archive: &Path, base: &Path) -> Result<(), String> {
    sevenz_rust2::decompress_file_with_extract_fn(archive, base, |entry, reader, path| {
        if is_junk(&entry.name().replace('\\', "/")) {
            // Entries in a solid block share one stream: drain the skipped
            // entry so the next one starts at the right offset.
            io::copy(reader, &mut io::sink())?;
            return Ok(true);
        }
        sevenz_rust2::default_entry_extract_fn(entry, reader, path)
    })
    .map_err(sevenz_error)
}

fn extract_rar(archive: &Path, base: &Path) -> Result<(), String> {
    unrar_ng::Archive::new(archive)
        .open_for_processing()
        .map_err(rar_error)?
        .extract_all(base)
        .map_err(rar_error)
}

fn open(path: &Path) -> Result<File, String> {
    File::open(path).map_err(|e| format!("Failed to open download: {}", e))
}

/// Folder name for an archive without a single top-level item: its file
/// name minus the extension, made safe for Windows.
fn wrapper_name(hint: &str) -> String {
    let stem = Path::new(hint)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = super::sanitize_filename(Some(stem), "");
    if name.is_empty() {
        "Downloaded song".into()
    } else {
        name
    }
}

const DAMAGED: &str = "The archive is damaged (CRC check failed). Try downloading it again.";
const DAMAGED_OR_INCOMPLETE: &str = "The archive is damaged or incomplete. Try downloading it again.";
const NOT_ARCHIVE: &str = "The download isn't a valid archive (it may be damaged or incomplete).";
const PASSWORD: &str = "This archive is password-protected, so YARGLE can't extract it.";

fn zip_error(e: zip::result::ZipError) -> String {
    match e {
        zip::result::ZipError::InvalidArchive(_) => NOT_ARCHIVE.into(),
        zip::result::ZipError::UnsupportedArchive(m) if m.to_ascii_lowercase().contains("password") => {
            PASSWORD.into()
        }
        other => format!("Couldn't extract the zip: {}", other),
    }
}

fn sevenz_error(e: sevenz_rust2::Error) -> String {
    use sevenz_rust2::Error as E;
    match e {
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => DAMAGED.into(),
        E::BadSignature(_) => NOT_ARCHIVE.into(),
        E::PasswordRequired | E::MaybeBadPassword(_) => PASSWORD.into(),
        // Decoder failures surface as I/O errors of these kinds.
        E::Io(ref io, _)
            if matches!(
                io.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ) =>
        {
            DAMAGED_OR_INCOMPLETE.into()
        }
        other => format!("Couldn't extract the 7z archive: {}", other),
    }
}

fn rar_error(e: unrar_ng::error::UnrarError) -> String {
    use unrar_ng::error::Code;
    match e.code {
        Code::BadData => DAMAGED.into(),
        Code::BadArchive | Code::UnknownFormat => NOT_ARCHIVE.into(),
        Code::MissingPassword | Code::BadPassword => PASSWORD.into(),
        _ => format!("Couldn't extract the RAR archive: {}", e),
    }
}
