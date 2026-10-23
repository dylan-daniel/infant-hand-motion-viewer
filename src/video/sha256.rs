use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Lowercase hex SHA-256 of the whole file at `path`, read in chunks.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&chunk[..read]);
    }
}
