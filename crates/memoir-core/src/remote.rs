//! The cloud side, as the sync engine sees it: a store of immutable files in
//! three areas, with a feed of changes.
//!
//! Files are never overwritten — every write creates a new file, and old
//! ones are deleted. Google Drive can't do compare-and-swap (it ignores
//! `If-Match`), so overwriting could silently lose an edit made on another
//! device; creating new files cannot.
//!
//! ```text
//! Memoir/            Area::Root     vault.json
//!   objects/         Area::Objects  <object id>.<revision id>.mmr
//!   blobs/           Area::Blobs    <blob id>.mmr
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

mod folder;
pub use folder::FolderProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    Root,
    Objects,
    Blobs,
}

impl Area {
    pub const ALL: [Area; 3] = [Area::Root, Area::Objects, Area::Blobs];
}

/// A file in the cloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    /// The provider's id for the file (not its name).
    pub id: String,
    pub area: Area,
    pub name: String,
    pub size: u64,
    /// Last modification, unix ms (as reported by the provider).
    pub modified: i64,
}

/// What changed since a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Files created (or changed) since the cursor that still exist.
    pub files: Vec<RemoteFile>,
    /// Ids of files deleted since the cursor.
    pub removed: Vec<String>,
    /// Where to continue next time.
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("not signed in to the cloud, or access was revoked — connect the account again")]
    Auth,
    #[error("network problem: {0}")]
    Network(String),
    #[error("the cloud service is busy, try again later")]
    Busy,
    #[error("the cloud storage is full")]
    Quota,
    #[error("not found in the cloud: {0}")]
    NotFound(String),
    #[error("cloud error: {0}")]
    Other(String),
}

impl ProviderError {
    /// Worth retrying later without user action.
    pub fn is_transient(&self) -> bool {
        matches!(self, ProviderError::Network(_) | ProviderError::Busy)
    }
}

pub type Result<T> = std::result::Result<T, ProviderError>;

/// A cloud storage service. Implementations retry transient failures
/// internally where sensible; errors that reach the caller are final for
/// this attempt.
pub trait StorageProvider: Send + Sync {
    /// Every file in an area.
    fn list(&self, area: Area) -> impl Future<Output = Result<Vec<RemoteFile>>> + Send;

    /// A cursor for "now": [`Self::changes`] from it reports later changes.
    fn cursor(&self) -> impl Future<Output = Result<String>> + Send;

    /// Changes in all areas since `cursor`.
    fn changes(&self, cursor: &str) -> impl Future<Output = Result<Changes>> + Send;

    fn download(&self, file_id: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Creates a new file (names need not be unique; nothing is replaced).
    fn upload(
        &self,
        area: Area,
        name: &str,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<RemoteFile>> + Send;

    fn delete(&self, file_id: &str) -> impl Future<Output = Result<()>> + Send;
}

/// In-memory provider for tests, with failure injection.
#[derive(Default)]
pub struct MemoryProvider {
    state: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    next_id: u64,
    files: BTreeMap<String, (RemoteFile, Vec<u8>)>,
    /// (sequence, file id, removed?)
    log: Vec<(u64, String, bool)>,
    /// Scripted outcomes of the next calls (`None` = succeed).
    failures: VecDeque<Option<ProviderError>>,
    calls: u64,
}

impl MemoryProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next calls fail with these errors, in order.
    pub fn fail_next(&self, errors: impl IntoIterator<Item = ProviderError>) {
        self.state
            .lock()
            .expect("lock")
            .failures
            .extend(errors.into_iter().map(Some));
    }

    /// Lets `skip` more calls succeed, then fails the one after with `error`.
    pub fn fail_after(&self, skip: usize, error: ProviderError) {
        let mut state = self.state.lock().expect("lock");
        state.failures.extend(std::iter::repeat_n(None, skip));
        state.failures.push_back(Some(error));
    }

    /// Drops any scripted failures that have not happened yet.
    pub fn clear_failures(&self) {
        self.state.lock().expect("lock").failures.clear();
    }

    /// Number of calls made (including failed ones).
    pub fn calls(&self) -> u64 {
        self.state.lock().expect("lock").calls
    }

    fn begin(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>> {
        let mut state = self.state.lock().expect("lock");
        state.calls += 1;
        match state.failures.pop_front().flatten() {
            Some(e) => Err(e),
            None => Ok(state),
        }
    }
}

impl StorageProvider for MemoryProvider {
    async fn list(&self, area: Area) -> Result<Vec<RemoteFile>> {
        let state = self.begin()?;
        Ok(state
            .files
            .values()
            .filter(|(f, _)| f.area == area)
            .map(|(f, _)| f.clone())
            .collect())
    }

    async fn cursor(&self) -> Result<String> {
        let state = self.begin()?;
        Ok(state.log.len().to_string())
    }

    async fn changes(&self, cursor: &str) -> Result<Changes> {
        let state = self.begin()?;
        let from: usize = cursor
            .parse()
            .map_err(|_| ProviderError::Other(format!("bad cursor {cursor:?}")))?;
        let mut changes = Changes {
            cursor: state.log.len().to_string(),
            ..Changes::default()
        };
        let mut seen = std::collections::HashSet::new();
        for (_, id, _) in state.log.iter().skip(from).rev() {
            if !seen.insert(id.clone()) {
                continue; // only the latest event per file counts
            }
            match state.files.get(id) {
                Some((file, _)) => changes.files.push(file.clone()),
                None => changes.removed.push(id.clone()),
            }
        }
        changes.files.reverse();
        changes.removed.reverse();
        Ok(changes)
    }

    async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let state = self.begin()?;
        state
            .files
            .get(file_id)
            .map(|(_, data)| data.clone())
            .ok_or_else(|| ProviderError::NotFound(file_id.to_owned()))
    }

    async fn upload(&self, area: Area, name: &str, data: Vec<u8>) -> Result<RemoteFile> {
        let mut state = self.begin()?;
        state.next_id += 1;
        let file = RemoteFile {
            id: format!("f{}", state.next_id),
            area,
            name: name.to_owned(),
            size: data.len() as u64,
            modified: state.next_id as i64,
        };
        let seq = state.log.len() as u64;
        state.log.push((seq, file.id.clone(), false));
        state.files.insert(file.id.clone(), (file.clone(), data));
        Ok(file)
    }

    async fn delete(&self, file_id: &str) -> Result<()> {
        let mut state = self.begin()?;
        if state.files.remove(file_id).is_none() {
            return Err(ProviderError::NotFound(file_id.to_owned()));
        }
        let seq = state.log.len() as u64;
        state.log.push((seq, file_id.to_owned(), true));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_provider_files_and_changes() {
        let p = MemoryProvider::new();
        let start = p.cursor().await.unwrap();
        let a = p
            .upload(Area::Objects, "a.r1.mmr", b"one".to_vec())
            .await
            .unwrap();
        let b = p
            .upload(Area::Blobs, "b.mmr", b"two".to_vec())
            .await
            .unwrap();

        assert_eq!(p.list(Area::Objects).await.unwrap(), vec![a.clone()]);
        assert_eq!(p.download(&b.id).await.unwrap(), b"two");

        let mid = p.cursor().await.unwrap();
        p.delete(&a.id).await.unwrap();
        let c = p
            .upload(Area::Objects, "a.r2.mmr", b"three".to_vec())
            .await
            .unwrap();

        let since_start = p.changes(&start).await.unwrap();
        assert_eq!(since_start.files, vec![b.clone(), c.clone()]);
        assert_eq!(since_start.removed, vec![a.id.clone()]);

        let since_mid = p.changes(&mid).await.unwrap();
        assert_eq!(since_mid.files, vec![c]);
        assert_eq!(since_mid.removed, vec![a.id.clone()]);

        let none = p.changes(&since_mid.cursor).await.unwrap();
        assert!(none.files.is_empty() && none.removed.is_empty());
        assert!(matches!(
            p.delete(&a.id).await,
            Err(ProviderError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn memory_provider_injects_failures() {
        let p = MemoryProvider::new();
        p.fail_next([
            ProviderError::Network("cable unplugged".into()),
            ProviderError::Auth,
        ]);
        assert!(p.list(Area::Root).await.unwrap_err().is_transient());
        assert_eq!(p.list(Area::Root).await.unwrap_err(), ProviderError::Auth);
        assert!(p.list(Area::Root).await.is_ok());
        assert_eq!(p.calls(), 3);
    }
}
