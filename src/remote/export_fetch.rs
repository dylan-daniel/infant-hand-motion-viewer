use crate::cache::DiskCache;
use crate::video::sha256::sha256_hex;

use super::{FileReply, RemoteClient};

/// The bytes of a remote `.hexport`. With a cache, the server is asked whether its copy still has the hash of the one
/// cached for this host and path, and sends the file only when it differs.
pub fn fetch_export(client: &RemoteClient, cache: Option<&DiskCache>, remote_path: &str) -> Result<Vec<u8>, String> {
    let Some(cache) = cache else {
        return client.fetch_file_bytes(remote_path);
    };
    let key = format!("{}:{remote_path}", client.config().host);
    let mut known = cache.known_export(&key);
    loop {
        match client.fetch_file_if_changed(remote_path, known.as_deref())? {
            FileReply::NotModified { hash } => match cache.read_export(&hash) {
                Some(bytes) => return Ok(bytes),
                // Gone or damaged since the index was written: ask again without a hash to get the file itself
                None => known = None,
            },
            FileReply::Fresh { hash, bytes } => {
                if sha256_hex(&bytes) == hash {
                    let _ = cache.store_export(&key, &hash, &bytes);
                }
                return Ok(bytes);
            }
        }
    }
}
