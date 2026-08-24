//! Format adapters. Every format normalises into `document::Book` for translation
//! and keeps a format-specific handle (`SourceDoc`) for byte-faithful write-back.

pub mod dom;
pub mod epub;
pub mod frontmatter;
pub mod placeholder;
mod txt;
pub mod typography;

use crate::config::OutputMode;
use crate::document::{Book, Format};
use crate::error::{CoreError, Result};
use std::path::Path;

pub enum SourceDoc {
    Epub(epub::EpubDoc),
    Txt(txt::TxtDoc),
}

/// Parse a file into translation IR + a reassembly handle.
pub fn extract(path: &Path) -> Result<(Book, SourceDoc)> {
    match Format::detect(path) {
        Some(Format::Epub) => {
            let (book, doc) = epub::extract(path)?;
            Ok((book, SourceDoc::Epub(doc)))
        }
        Some(Format::Txt) => {
            let (book, doc) = txt::extract(path)?;
            Ok((book, SourceDoc::Txt(doc)))
        }
        None => Err(CoreError::UnsupportedFormat(path.display().to_string())),
    }
}

/// Write the translated (and/or annotated) book back out. `lang` labels
/// translated nodes in bilingual mode (fonts / TTS / RTL); `note_lang` labels
/// annotation blocks (None = no lang attribute on notes).
pub fn write(
    doc: &SourceDoc,
    book: &Book,
    out: &Path,
    mode: OutputMode,
    lang: &str,
    note_lang: Option<&str>,
) -> Result<()> {
    match doc {
        SourceDoc::Epub(d) => epub::write(d, book, out, mode, lang, note_lang),
        SourceDoc::Txt(d) => txt::write(d, book, out, mode, lang),
    }
}

/// Write a complete output without ever opening the destination for writing.
///
/// The translation may run for hours, so a path check at startup is not a
/// sufficient symlink defence. We write through an owner-only, create-new file
/// in the destination directory and atomically rename it into place. A symlink
/// swapped in while the model runs is replaced as a directory entry; its target
/// is never followed. A crash before rename leaves the previous output intact.
pub(crate) fn atomic_write(out: &Path, bytes: &[u8]) -> Result<()> {
    use std::ffi::OsString;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    let base = out
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("output"));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut created = None;
    for attempt in 0..128_u64 {
        let mut name = OsString::from(".");
        name.push(base);
        name.push(format!(
            ".translatus-tmp-{}-{nonce}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed) + attempt
        ));
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                created = Some((path, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let (temp, mut file) = created.ok_or_else(|| {
        CoreError::Other(format!(
            "could not create a private temporary output beside {}",
            out.display()
        ))
    })?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, out)?;
        #[cfg(unix)]
        if let Ok(parent) = std::fs::File::open(dir) {
            let _ = parent.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(Into::into)
}

#[cfg(test)]
mod atomic_write_tests {
    use super::atomic_write;

    #[cfg(unix)]
    #[test]
    fn destination_swapped_to_symlink_is_replaced_without_touching_victim() {
        use std::os::unix::fs::{symlink, MetadataExt};

        let dir = std::env::temp_dir().join(format!(
            "translatus-atomic-output-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        let victim = dir.join("victim.txt");
        let output = dir.join("book.out.txt");
        std::fs::write(&victim, "sentinel").expect("victim");

        // Simulate an attacker replacing a previously checked output path while
        // a long translation is in progress.
        symlink(&victim, &output).expect("swap in symlink");
        atomic_write(&output, b"translated").expect("atomic output");

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "sentinel");
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "translated");
        assert!(!std::fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::metadata(&output).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&output).unwrap().nlink(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
