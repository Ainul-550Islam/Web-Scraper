//! Shared per-export write budget, including adapter formats and bundles.
//! Thread-local scope is safe here: export and adapter I/O are synchronous;
//! different blocking jobs/parallel tests get independent budgets.
use crate::budget::Budget;
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

const DEFAULT_CAP: u64 = 256 * 1024 * 1024;
thread_local! { static CURRENT: RefCell<Option<Arc<Budget>>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<Arc<Budget>>);
pub(crate) fn begin(cap: u64) -> Scope {
    Scope(CURRENT.with(|b| {
        b.replace(Some(Arc::new(Budget::new(
            "export_write_bytes",
            cap.clamp(1, DEFAULT_CAP),
        ))))
    }))
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|b| {
            b.replace(self.0.take());
        });
    }
}
pub(crate) fn remaining() -> u64 {
    CURRENT.with(|b| {
        b.borrow()
            .as_ref()
            .map_or(DEFAULT_CAP, |b| b.limit().saturating_sub(b.used()))
    })
}
pub(crate) fn charge(n: u64) -> io::Result<()> {
    let accepted = CURRENT.with(|b| b.borrow().as_ref().is_none_or(|b| b.try_take(n)));
    if accepted {
        Ok(())
    } else {
        Err(io::Error::other(
            "export output byte budget exceeded; split the source or select fewer formats",
        ))
    }
}
pub(crate) struct BudgetedFile {
    file: File,
    budget: Option<Arc<Budget>>,
}
pub(crate) fn wrap(file: File) -> BudgetedFile {
    BudgetedFile {
        file,
        budget: CURRENT.with(|b| b.borrow().clone()),
    }
}
pub(crate) fn create(path: &Path) -> io::Result<BudgetedFile> {
    crate::work_cancel::check()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    crate::fsutil::reject_symlink_components(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output needs a filename"))?;
    let file = crate::fsutil::open_within(parent, &parent.join(name), true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata()?.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "export must not overwrite a hard-linked file",
            ));
        }
    }
    file.set_len(0)?;
    Ok(wrap(file))
}
pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
impl Write for BudgetedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        crate::work_cancel::check()?;
        if self
            .budget
            .as_ref()
            .is_some_and(|b| !b.try_take(bytes.len() as u64))
        {
            return Err(io::Error::other(
                "export output byte budget exceeded; split the source or select fewer formats",
            ));
        }
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for BudgetedFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.file.seek(from)
    }
}
impl BudgetedFile {
    pub(crate) fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()
    }
}
