//! Private filesystem helpers. Unix walks start from an opened directory
//! descriptor, not a re-resolved pathname, and never follow symlink children.
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Component, Path};

pub(crate) fn open_within(root: &Path, path: &Path, create: bool) -> io::Result<File> {
    open_mode_within(root, path, create, false)
}

/// Read one regular, non-symlink file under its parent directory with a hard
/// byte cap. Unix traversal is rooted at an opened directory descriptor.
pub(crate) fn read_bounded(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "input needs a filename"))?;
    let rooted_path = parent.join(name);
    let file = open_within(parent, &rooted_path, false)?;
    let length = file.metadata()?.len();
    if length > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("input exceeds the {max_bytes}-byte limit"),
        ));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("input exceeds the {max_bytes}-byte limit"),
        ));
    }
    Ok(bytes)
}

/// Kernel-atomic append, private creation, and no symlink/special-file adoption.
pub(crate) fn open_append(path: &Path) -> io::Result<File> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_directory_nofollow(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output needs a filename"))?;
    let file = open_mode_within(parent, &parent.join(name), true, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata()?.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output must not adopt a hard-linked file",
            ));
        }
    }
    Ok(file)
}

/// Create or replace an app-owned state file with owner-only permissions.
/// The temporary is exclusive and no-follow; on Unix both creation and the
/// final atomic rename are anchored to one open parent-directory descriptor.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output needs a filename"))?;
    ensure_directory_nofollow(parent)?;

    #[cfg(unix)]
    {
        let directory = open_directory_nofollow(parent)?;
        let display_name = name.to_string_lossy();
        let (tmp_name, mut file) = loop {
            let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let tmp_name = std::ffi::OsString::from(format!(
                ".{display_name}.tmp.{}.{}",
                std::process::id(),
                nonce
            ));
            match create_new_private_in_directory(&directory, &tmp_name) {
                Ok(file) => break (tmp_name, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            rename_name_in_directory(&directory, &tmp_name, name)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = unlink_name_in_directory(&directory, &tmp_name);
        }
        result
    }

    #[cfg(not(unix))]
    {
        let display_name = name.to_string_lossy();
        let (tmp, mut file) = loop {
            let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let tmp = parent.join(format!(
                ".{display_name}.tmp.{}.{}",
                std::process::id(),
                nonce
            ));
            match create_new_private(&tmp) {
                Ok(file) => break (tmp, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            rename_in_directory(parent, &tmp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

/// Refuse symlinked parents both before and after directory creation. As with
/// all portable pathname checks this does not claim race-free protection on
/// non-Unix platforms; Unix file opens themselves are anchored with openat.
pub(crate) fn ensure_directory_nofollow(path: &Path) -> io::Result<()> {
    reject_symlink_components(path)?;
    std::fs::create_dir_all(path)?;
    reject_symlink_components(path)
}

/// Remove a file name without resolving a symlinked parent path. A final
/// symlink is unlinked as a link; it is never followed to its target.
pub(crate) fn remove_file_nofollow(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "input needs a filename"))?;
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in filename"))?;
        let directory = open_directory_nofollow(parent)?;
        // SAFETY: directory is a live fd and name is NUL-terminated.
        let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = directory.sync_all();
        Ok(())
    }
    #[cfg(not(unix))]
    {
        reject_symlink_components(parent)?;
        std::fs::remove_file(path)
    }
}

#[cfg(unix)]
fn create_new_private_in_directory(directory: &File, name: &std::ffi::OsStr) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in temporary name"))?;
    // SAFETY: directory is a live descriptor; the basename is NUL-terminated,
    // and a successful openat returns a fresh descriptor adopted exactly once.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat returned a fresh owned descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "temporary output must be a private regular file",
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(unix)]
fn rename_name_in_directory(
    directory: &File,
    from: &std::ffi::OsStr,
    to: &std::ffi::OsStr,
) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(from.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in temporary name"))?;
    let to = CString::new(to.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in output name"))?;
    // SAFETY: the directory descriptor is live and both names are NUL-terminated
    // basenames, so the atomic rename is anchored to one open directory.
    let result = unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let _ = directory.sync_all();
    Ok(())
}

#[cfg(unix)]
fn unlink_name_in_directory(directory: &File, name: &std::ffi::OsStr) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in temporary name"))?;
    // SAFETY: the directory descriptor is live and name is a NUL-terminated
    // basename; unlinkat removes only that directory entry.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_new_private(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "temporary output must be a regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn rename_in_directory(parent: &Path, from: &Path, to: &Path) -> io::Result<()> {
    reject_symlink_components(parent)?;
    std::fs::rename(from, to)
}

fn open_mode_within(root: &Path, path: &Path, create: bool, append: bool) -> io::Result<File> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "path is outside its root"))?;
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "path must name a file beneath its root",
        ));
    }
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let mut directory = open_directory_nofollow(root)?;
        for (i, part) in parts.iter().enumerate() {
            let Component::Normal(name) = part else {
                unreachable!()
            };
            let name = CString::new(name.as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
            let last = i + 1 == parts.len();
            let flags = if last {
                libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | if create {
                        libc::O_RDWR | libc::O_CREAT | if append { libc::O_APPEND } else { 0 }
                    } else {
                        libc::O_RDONLY
                    }
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
            };
            // SAFETY: directory owns a live fd, name is NUL-terminated, and
            // the returned fd is adopted exactly once after checking failure.
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful openat returned a fresh owned descriptor.
            let opened = unsafe { File::from_raw_fd(fd) };
            if last {
                if !opened.metadata()?.is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "not a regular file",
                    ));
                }
                return Ok(opened);
            }
            directory = opened;
        }
        unreachable!()
    }
    #[cfg(not(unix))]
    {
        // Portable fallback rejects every detectable symlink component.
        // Windows reparse-point race guarantees are not claimed by this helper.
        reject_symlink_components(root)?;
        let mut current = root.to_path_buf();
        if std::fs::symlink_metadata(&current)?
            .file_type()
            .is_symlink()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "symlink root",
            ));
        }
        for part in parts {
            current.push(part.as_os_str());
            match std::fs::symlink_metadata(&current) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "symlink path",
                    ))
                }
                Ok(_) => {}
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(create)
            .create(create)
            .append(append)
            .truncate(false)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "not a regular file",
            ));
        }
        Ok(file)
    }
}

/// Walk the approved root itself by directory fd too. O_NOFOLLOW on
/// `.open(root)` alone protects only the last component, not its ancestors.
#[cfg(unix)]
fn open_directory_nofollow(root: &Path) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let mut dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(if root.is_absolute() {
            Path::new("/")
        } else {
            Path::new(".")
        })?;
    for component in root.components() {
        let name = match component {
            Component::Normal(name) => name,
            // The root itself is explicitly approved; parent navigation
            // is allowed there, never in an untrusted relative child path.
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::CurDir | Component::RootDir => continue,
            Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported root prefix",
                ))
            }
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in root"))?;
        // SAFETY: the live parent fd and NUL-terminated component are
        // owned here, and every successful fresh fd is adopted exactly once.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a fresh owned descriptor.
        dir = unsafe { File::from_raw_fd(fd) };
    }
    Ok(dir)
}

/// Only use for app-owned directories, never a user's original output root.
pub(crate) fn prepare_private_directory(path: &Path) -> io::Result<()> {
    reject_symlink_components(path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let directory = open_directory_nofollow(path)?;
        // SAFETY: geteuid has no preconditions and does not access pointers.
        let uid = unsafe { libc::geteuid() };
        if directory.metadata()?.uid() != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "managed directory must be owned by the service user",
            ));
        }
        directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn reject_symlink_components(path: &Path) -> io::Result<()> {
    let mut current = std::path::PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "symlink directory path is not allowed",
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Refuse SQLite journal sidecars that could redirect SQLite outside the
/// configured output directory or alias another file. SQLite's `NOFOLLOW`
/// open flag protects the database pathname itself; sidecars need their own
/// preflight because SQLite creates/opens them through its VFS.
pub(crate) fn reject_sqlite_sidecars(path: &Path) -> io::Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = std::path::PathBuf::from(name);
        reject_symlink_components(&sidecar)?;
        match std::fs::symlink_metadata(&sidecar) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SQLite journal sidecar must be a regular non-symlink file",
                ));
            }
            #[cfg(unix)]
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "SQLite journal sidecar must not be hard-linked",
                    ));
                }
            }
            #[cfg(not(unix))]
            Ok(_) => {}
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod phase2_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn phase2_ancestor_symlink_cannot_redirect_an_anchored_root() {
        let t = std::env::temp_dir().join(format!("harvest-phase2-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("private/deep")).unwrap();
        std::fs::write(t.join("private/deep/data"), b"private").unwrap();
        symlink(t.join("private"), t.join("alias")).unwrap();
        let root = t.join("alias/deep");
        let result = open_within(&root, &root.join("data"), false);
        assert!(
            result.is_err(),
            "nofollow must cover root ancestors, not only its last component"
        );
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn private_atomic_write_replaces_a_destination_symlink_not_its_target() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let t = std::env::temp_dir().join(format!("harvest-private-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let victim = t.join("victim");
        let output = t.join("state.jsonl");
        std::fs::write(&victim, b"keep me").unwrap();
        symlink(&victim, &output).unwrap();

        write_private_atomic(&output, b"private state\n").unwrap();

        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
        assert_eq!(std::fs::read(&output).unwrap(), b"private state\n");
        let meta = std::fs::metadata(&output).unwrap();
        assert_eq!(meta.nlink(), 1);
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn private_atomic_write_rejects_a_symlinked_parent() {
        let t = std::env::temp_dir().join(format!("harvest-private-parent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("real")).unwrap();
        symlink(t.join("real"), t.join("alias")).unwrap();

        let result = write_private_atomic(&t.join("alias/state.jsonl"), b"secret");

        assert!(result.is_err());
        assert!(!t.join("real/state.jsonl").exists());
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn safe_removal_unlinks_only_the_symlink_name() {
        let t = std::env::temp_dir().join(format!("harvest-safe-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let victim = t.join("victim");
        let link = t.join("state");
        std::fs::write(&victim, b"keep").unwrap();
        symlink(&victim, &link).unwrap();

        remove_file_nofollow(&link).unwrap();

        assert!(!link.exists());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        std::fs::remove_dir_all(t).unwrap();
    }
}
