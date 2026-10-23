use serde::{Deserialize, Serialize};

use crate::cache::DiskCache;
use crate::data::{GraspEventSet, GraspFile};
use crate::remote::{RemoteClient, fetch_export};

/// A grasp-events CSV the user added: a path on this machine or on the connected server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraspSource {
    pub path: String,
    #[serde(default)]
    pub remote: bool,
}

/// A source after an attempt to load it.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub source: GraspSource,
    pub result: Result<GraspFile, String>,
}

/// Loads every source in order; a source that fails keeps its error and does not stop the others.
pub fn load_sources(sources: &[GraspSource], client: &RemoteClient, cache: Option<&DiskCache>) -> Vec<LoadedSource> {
    sources
        .iter()
        .map(|source| {
            let bytes = if source.remote {
                fetch_export(client, cache, &source.path)
            } else {
                std::fs::read(&source.path).map_err(|err| format!("{}: {err}", source.path))
            };
            let result = bytes.and_then(|bytes| GraspFile::from_bytes(&bytes, &source.path).map_err(|e| e.to_string()));
            LoadedSource {
                source: source.clone(),
                result,
            }
        })
        .collect()
}

/// The rows of every file that loaded, the first source winning a trial it shares with a later one.
pub fn merge_loaded(loaded: &[LoadedSource]) -> GraspEventSet {
    GraspEventSet::merge(loaded.iter().filter_map(|entry| entry.result.as_ref().ok()))
}
