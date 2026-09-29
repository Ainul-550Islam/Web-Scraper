//! HMAC-SHA256 detached attestation for client-facing evidence.
//!
//! The problem this closes: `audit.json` and `manifest.json` are proof —
//! reconciled request receipts, per-file sha256s, the policy hash that ties
//! a delivery to the politeness config it ran under. As plain files they
//! prove nothing once they leave the building: anyone who can touch them can
//! edit them. With an HMAC over the exact artifact bytes, held under a key
//! shared out-of-band between the crawl operator and whoever verifies, a
//! recipient can prove (a) nobody modified the file after signing and (b) the
//! signer holds the shared key. No PKI, no cert lifecycle, no cloud KMS —
//! deliberately the smallest thing that is actually enforceable.
//!
//! Format: a detached `<artifact>.sig` file containing one lowercase hex
//! HMAC-SHA256 digest (plus a trailing newline). Detached means the signed
//! artifact is never rewritten by us, so mirrors/rclone copies and the
//! manifest's own sha256s stay byte-stable.
//!
//! Key file: one line of hex, at least 32 bytes of key material (longer is
//! fine; HMAC pre-hashes anything over the SHA-256 block size, so oversized
//! keys remain safe, but we cap at 256 bytes to keep files honest).
//! `Key::generate` produces 32 fresh bytes from /dev/urandom with mode 0600
//! and refuses to overwrite an existing key.

use std::fs;
use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_digit(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => bail!("invalid hex digit {:?}", c as char),
    }
}

fn from_hex(s: &str) -> Result<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        bail!("hex string has odd length {}", b.len());
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push(hex_digit(b[i])? << 4 | hex_digit(b[i + 1])?);
        i += 2;
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Key {
    secret: Vec<u8>,
}

impl Key {
    /// Load a key file. Only the first line is considered; surrounding
    /// whitespace is stripped. Hex may be upper- or lowercase.
    pub fn load(path: &Path) -> Result<Key> {
        let txt =
            fs::read_to_string(path).with_context(|| format!("read key {}", path.display()))?;
        let line = txt.lines().next().unwrap_or_default().trim();
        if line.is_empty() {
            bail!(
                "key file {} is empty; create one with `harvest attest --gen-key {}`",
                path.display(),
                path.display()
            );
        }
        let secret = from_hex(line)
            .with_context(|| format!("parse key {} (expected one line of hex)", path.display()))?;
        if secret.len() < 32 {
            bail!(
                "key {} has only {} bytes of key material; 32 required \
                 (generate one with `harvest attest --gen-key`)",
                path.display(),
                secret.len()
            );
        }
        if secret.len() > 256 {
            bail!(
                "key {} has {} bytes; the maximum is 256",
                path.display(),
                secret.len()
            );
        }
        Ok(Key { secret })
    }

    /// Direct key bytes — for tests and callers that manage their own
    /// material. No length policy beyond "non-empty" (HMAC's own rules
    /// still apply at construction).
    pub fn from_secret(secret: Vec<u8>) -> Result<Key> {
        if secret.is_empty() {
            bail!("empty HMAC secret");
        }
        Ok(Key { secret })
    }

    /// Write a fresh 32-byte key as hex at `path` with mode 0600. Never
    /// overwrites: an existing key is data an operator owns, not ours to
    /// clobber.
    pub fn generate(path: &Path) -> Result<()> {
        if path.exists() {
            bail!("refusing to overwrite existing key: {}", path.display());
        }
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).with_context(|| format!("create dir {}", p.display()))?;
        }
        let mut raw = [0u8; 32];
        fs::File::open("/dev/urandom")
            .context("open /dev/urandom")?
            .read_exact(&mut raw)
            .context("read /dev/urandom")?;
        let body = format!("{}\n", to_hex(&raw));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("create key {}", path.display()))?;
        std::io::Write::write_all(&mut f, body.as_bytes())
            .with_context(|| format!("write key {}", path.display()))?;
        Ok(())
    }

    fn mac(&self) -> Result<HmacSha256> {
        HmacSha256::new_from_slice(&self.secret).context("HMAC key length")
    }

    /// Lowercase hex HMAC-SHA256 over `data`.
    pub fn sign_bytes(&self, data: &[u8]) -> Result<String> {
        let mut m = self.mac()?;
        m.update(data);
        Ok(to_hex(&m.finalize().into_bytes()))
    }

    /// Constant-time verification. An error here means exactly one of: the
    /// artifact changed after signing, or the verifier's key differs from
    /// the signer's.
    pub fn verify_bytes(&self, data: &[u8], sig: &str) -> Result<()> {
        let want =
            from_hex(sig.trim()).context("signature must be a single hex HMAC-SHA256 digest")?;
        let mut m = self.mac()?;
        m.update(data);
        m.verify_slice(&want).map_err(|_| {
            anyhow!("HMAC verification FAILED: the artifact was modified after signing, or the key is different")
        })
    }

    /// Sign `file` by writing `<file>.sig` over its exact bytes. Returns the
    /// signature path.
    pub fn sign_file(&self, file: &Path) -> Result<PathBuf> {
        let data = fs::read(file).with_context(|| format!("read {}", file.display()))?;
        let sig = self.sign_bytes(&data)?;
        let p = sig_path(file);
        fs::write(&p, format!("{sig}\n")).with_context(|| format!("write {}", p.display()))?;
        Ok(p)
    }

    /// Verify `file` against the digest stored in `sig_file`.
    pub fn verify_file(&self, file: &Path, sig_file: &Path) -> Result<()> {
        let data = fs::read(file).with_context(|| format!("read {}", file.display()))?;
        let sig =
            fs::read_to_string(sig_file).with_context(|| format!("read {}", sig_file.display()))?;
        self.verify_bytes(&data, &sig)
    }
}

/// `out/audit.json` -> `out/audit.json.sig`.
pub fn sig_path(file: &Path) -> PathBuf {
    let mut name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".sig");
    file.with_file_name(name)
}

#[cfg(test)]
mod tests;
