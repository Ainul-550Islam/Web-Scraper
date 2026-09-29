//! ZIP bundling for deliveries — one downloadable package per dataset.
//!
//! A bundle is the *same* bytes the folder export produced, plus a manifest
//! that names every member and a `checksums.txt` a client can feed to
//! `sha256sum -c`. Nothing is re-rendered for the archive, so a verified folder
//! and a verified bundle can never disagree about the data.
//!
//! Properties that matter for a paid deliverable:
//! * **Streaming.** Members are copied in 64 KiB chunks; a 4 GB export does
//!   not become 4 GB of RAM. Only the central directory is buffered, and it is
//!   bounded by the member count.
//! * **Atomic.** The archive is written to a sibling temp file, flushed, then
//!   renamed into place. A crashed run leaves no `.zip` at all — never a
//!   truncated one that looks finished (§34).
//! * **Path-safe.** Every member name goes through [`safe_archive_path`],
//!   which refuses absolute paths, drive letters, `..` traversal, backslash
//!   separators, control characters and over-long names. Refusal is an error,
//!   not a silent rewrite.
//! * **Deterministic.** Fixed DOS timestamps, fixed entry order (the order
//!   given), stored (uncompressed) members: same inputs ⇒ same bytes.
//!
//! The ZIP writer here is also the container used by the XLSX exporter
//! (`crate::export::xlsx`), so there is exactly one ZIP implementation in the
//! crate.

use std::fs::File;
use std::io::{BufReader, Read, Write};

/// Small adapter so file members can be buffered at [`COPY_CHUNK`] without
/// special-casing the reader construction.
trait Buffered {
    fn with_capacity_helper(self) -> BufReader<File>;
}
impl Buffered for File {
    fn with_capacity_helper(self) -> BufReader<File> {
        BufReader::with_capacity(COPY_CHUNK, self)
    }
}

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Bytes copied per read: small enough to keep memory flat, large enough to
/// stay out of syscall overhead.
pub const COPY_CHUNK: usize = 64 * 1024;
/// ZIP's 32-bit size ceiling. Refused explicitly rather than truncated.
pub const ZIP32_MAX: u64 = u32::MAX as u64;

const LOCAL_SIG: u32 = 0x04034b50;
const CENTRAL_SIG: u32 = 0x02014b50;
const EOCD_SIG: u32 = 0x06054b50;
/// General-purpose bit 11: the name is UTF-8.
const FLAG_UTF8: u16 = 0x0800;
/// 1980-01-01 00:00 — a fixed, valid DOS timestamp (determinism).
const DOS_TIME: u16 = 0x0000;
const DOS_DATE: u16 = 0x0021;

const fn build_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = build_crc_table();

/// Streaming CRC-32 (IEEE 802.3, the ZIP variant).
#[derive(Debug, Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self(0xFFFF_FFFF)
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for b in bytes {
            let idx = ((self.0 ^ (*b as u32)) & 0xFF) as usize;
            self.0 = CRC_TABLE[idx] ^ (self.0 >> 8);
        }
    }

    pub fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

/// One-file convenience wrapper around [`Crc32`].
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(bytes);
    c.finish()
}

/// One member of a bundle, as recorded in the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub bytes: u64,
    pub crc: u32,
}

/// Result of writing an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub entries: Vec<ZipEntry>,
}

/// Reject unsafe archive member names. Refusal — not silent sanitizing — is
/// the point: a name that cannot be expressed safely should fail the export
/// so the operator fixes the source.
pub fn safe_archive_path(name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("archive member name is empty");
    }
    if name.len() > 255 {
        bail!("archive member name is too long ({} > 255)", name.len());
    }
    if name.contains('\\') {
        bail!("archive member name contains a backslash: {name:?}");
    }
    if name.starts_with('/') || name.starts_with('.') && name.starts_with("./") {
        bail!("archive member name must be relative: {name:?}");
    }
    if name.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f) {
        bail!("archive member name contains control characters: {name:?}");
    }
    if let Some(colon) = name.find(':') {
        if colon == 1 {
            bail!("archive member name looks like a drive letter: {name:?}");
        }
    }
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            bail!("archive member name has an empty path segment: {name:?}");
        }
        if part == ".." {
            bail!("archive member name traverses upwards: {name:?}");
        }
        if part.len() > 255 {
            bail!("archive member path segment too long: {name:?}");
        }
    }
    Ok(name.to_string())
}

/// `sha256sum`-compatible text block, sorted by member name.
pub fn checksums_txt(entries: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (name, sha) in sorted {
        out.push_str(&format!("{sha}  {name}\n"));
    }
    out
}

/// The bundle manifest: every member, its size and digest, plus the dataset
/// identity the folder export recorded. `manifest.json` cannot contain its own
/// digest (it would have to be written twice), so that field says so instead
/// of carrying a placeholder.
pub fn bundle_manifest(
    dataset: &str,
    generated_ts: i64,
    records: u64,
    entries: &[(String, u64, String)],
    signature: Option<&str>,
) -> Value {
    json!({
        "v": 2,
        "kind": "harvest-bundle",
        "dataset": dataset,
        "generated_ts": generated_ts,
        "records": records,
        "files": entries
            .iter()
            .map(|(name, bytes, sha)| json!({"name": name, "bytes": bytes, "sha256": sha}))
            .collect::<Vec<_>>(),
        "self": "manifest.json is not listed in `files`; its digest is published in the export report and in checksums.txt",
        "signature": signature,
    })
}

/// A member to archive: either bytes already in memory (XLSX parts) or a file
/// streamed from disk (delivery bundles). One writer serves both, so there is
/// no second copy of the ZIP layout to keep in sync.
#[derive(Debug, Clone, Copy)]
pub enum Member<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
}

/// Stream every member from disk into one stored-mode ZIP at `out`.
pub fn write_zip_from_files(out: &Path, files: &[(String, PathBuf)]) -> Result<ZipReport> {
    let members: Vec<(String, Member<'_>)> = files
        .iter()
        .map(|(name, path)| (name.clone(), Member::File(path.as_path())))
        .collect();
    write_zip(out, &members)
}

/// Temp path next to the final archive, so the final rename is atomic on one
/// filesystem.
fn sibling_tmp(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "bundle".to_string());
    out.with_file_name(format!(".{name}.tmp"))
}

/// Write a stored-mode ZIP at `out` from `members`, in order. Duplicates and
/// unsafe names are refused before any byte is written.
pub fn write_zip(out: &Path, members: &[(String, Member<'_>)]) -> Result<ZipReport> {
    if members.is_empty() {
        bail!("refusing to write an empty archive");
    }
    let mut names: Vec<String> = Vec::with_capacity(members.len());
    for (name, _) in members {
        let safe = safe_archive_path(name)?;
        if names.contains(&safe) {
            bail!("duplicate archive member: {safe}");
        }
        names.push(safe);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let tmp = sibling_tmp(out);
    let mut w = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut central: Vec<u8> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::with_capacity(members.len());
    let mut offset: u64 = 0;
    let mut sha_all = Sha256::new();
    let mut total: u64 = 0;

    for (idx, (_name, member_src)) in members.iter().enumerate() {
        let member = &names[idx];
        let opened: Option<File> = match member_src {
            Member::Bytes(_) => None,
            Member::File(p) => {
                Some(File::open(p).with_context(|| format!("open {}", p.display()))?)
            }
        };
        let size: u64 = match member_src {
            Member::Bytes(b) => b.len() as u64,
            Member::File(p) => std::fs::metadata(p)
                .with_context(|| format!("stat {}", p.display()))?
                .len(),
        };
        if size > ZIP32_MAX {
            bail!(
                "member {member} is {} bytes; ZIP32 archives cap at {}",
                size,
                ZIP32_MAX
            );
        }
        let mut reader: Option<BufReader<File>> = opened.map(Buffered::with_capacity_helper);
        let name_bytes = member.as_bytes();
        let member_start = offset;

        let mut local: Vec<u8> = Vec::with_capacity(30 + name_bytes.len());
        local.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes()); // stored
        local.extend_from_slice(&DOS_TIME.to_le_bytes());
        local.extend_from_slice(&DOS_DATE.to_le_bytes());
        let crc_slot = local.len();
        local.extend_from_slice(&0u32.to_le_bytes()); // crc, filled below
        let size_slot = local.len();
        local.extend_from_slice(&(size as u32).to_le_bytes());
        local.extend_from_slice(&(size as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes()); // extra len
        local.extend_from_slice(name_bytes);
        w.write_all(&local)?;

        let mut crc = Crc32::new();
        let mut buf = [0u8; COPY_CHUNK];
        let mut written: u64 = 0;
        match (&mut reader, member_src) {
            (Some(r), _) => loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                crc.update(&buf[..n]);
                sha_all.update(&buf[..n]);
                w.write_all(&buf[..n])?;
                written += n as u64;
            },
            (None, Member::Bytes(b)) => {
                crc.update(b);
                sha_all.update(b);
                w.write_all(b)?;
                written = b.len() as u64;
            }
            (None, Member::File(_)) => unreachable!("file members always open a reader"),
        }
        if written != size {
            bail!(
                "member {member} changed while being archived ({} -> {})",
                size,
                written
            );
        }
        let crc = crc.finish();
        // The local header reserves the CRC and both size fields before the
        // bytes are known; patch them in place (a single backwards seek per
        // member — the member body itself is never re-read).
        patch_local_header(&mut w, member_start, crc_slot, size_slot, crc, size as u32)
            .with_context(|| format!("patch local header for {member}"))?;

        central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(size as u32).to_le_bytes());
        central.extend_from_slice(&(size as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&(offset as u32).to_le_bytes());
        central.extend_from_slice(name_bytes);

        entries.push(ZipEntry {
            name: member.clone(),
            bytes: size,
            crc,
        });
        offset += local.len() as u64 + size;
        total += local.len() as u64 + size;
    }

    let cd_offset = offset;
    w.write_all(&central)?;
    total += central.len() as u64;

    let mut eocd: Vec<u8> = Vec::with_capacity(22);
    eocd.extend_from_slice(&EOCD_SIG.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes());
    eocd.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    eocd.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    eocd.extend_from_slice(&(central.len() as u32).to_le_bytes());
    eocd.extend_from_slice(&(cd_offset as u32).to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes());
    w.write_all(&eocd)?;
    total += eocd.len() as u64;
    w.flush()?;
    w.sync_all()?;
    drop(w);

    std::fs::rename(&tmp, out)
        .with_context(|| format!("rename {} -> {}", tmp.display(), out.display()))?;

    // Digest the archive *file* (not just its members): that is the value a
    // client can re-hash after a download. Streamed, so memory stays flat.
    let mut file_hash = Sha256::new();
    let mut reader = BufReader::with_capacity(COPY_CHUNK, File::open(out)?);
    let mut buf = [0u8; COPY_CHUNK];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file_hash.update(&buf[..n]);
    }
    let _ = sha_all;

    Ok(ZipReport {
        path: out.to_path_buf(),
        bytes: total,
        sha256: format!("{:x}", file_hash.finalize()),
        entries,
    })
}

/// Rewrite the CRC and sizes in a member's local file header, then restore the
/// write position. Seeks within the header only — the member's bytes stay on
/// disk, never in memory.
fn patch_local_header(
    w: &mut File,
    member_start: u64,
    crc_slot: usize,
    size_slot: usize,
    crc: u32,
    size: u32,
) -> Result<()> {
    use std::io::Seek;
    let pos = w.stream_position().context("tell")?;
    w.seek(std::io::SeekFrom::Start(member_start + crc_slot as u64))?;
    w.write_all(&crc.to_le_bytes())?;
    w.seek(std::io::SeekFrom::Start(member_start + size_slot as u64))?;
    w.write_all(&size.to_le_bytes())?;
    w.write_all(&size.to_le_bytes())?;
    w.seek(std::io::SeekFrom::Start(pos))?;
    Ok(())
}
