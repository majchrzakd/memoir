//! A folder on disk laid out like the cloud folder (`vault.json`,
//! `objects/`, `blobs/`), e.g. a downloaded copy of the Memoir folder from
//! Google Drive. Used by the recovery tool; file ids are paths relative to
//! the folder.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{Area, Changes, ProviderError, RemoteFile, Result, StorageProvider};

pub struct FolderProvider {
    root: PathBuf,
}

impl FolderProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, area: Area) -> PathBuf {
        match area {
            Area::Root => self.root.clone(),
            Area::Objects => self.root.join("objects"),
            Area::Blobs => self.root.join("blobs"),
        }
    }

    fn prefix(area: Area) -> &'static str {
        match area {
            Area::Root => "",
            Area::Objects => "objects/",
            Area::Blobs => "blobs/",
        }
    }

    /// The path of a file id, refusing anything that leaves the folder.
    fn path(&self, file_id: &str) -> Result<PathBuf> {
        let relative = Path::new(file_id);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ProviderError::NotFound(file_id.to_owned()));
        }
        Ok(self.root.join(relative))
    }
}

fn io_error(e: io::Error, what: &str) -> ProviderError {
    match e.kind() {
        io::ErrorKind::NotFound => ProviderError::NotFound(what.to_owned()),
        _ => ProviderError::Other(format!("{what}: {e}")),
    }
}

impl StorageProvider for FolderProvider {
    async fn list(&self, area: Area) -> Result<Vec<RemoteFile>> {
        let dir = self.dir(area);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(e, &dir.display().to_string())),
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| io_error(e, &dir.display().to_string()))?;
            let meta = entry
                .metadata()
                .map_err(|e| io_error(e, &entry.path().display().to_string()))?;
            if !meta.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            files.push(RemoteFile {
                id: format!("{}{name}", Self::prefix(area)),
                area,
                name,
                size: meta.len(),
                modified,
            });
        }
        files.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(files)
    }

    async fn cursor(&self) -> Result<String> {
        Ok("folder".to_owned())
    }

    /// A folder has no change feed: everything counts as changed.
    async fn changes(&self, _cursor: &str) -> Result<Changes> {
        let mut files = Vec::new();
        for area in Area::ALL {
            files.extend(self.list(area).await?);
        }
        Ok(Changes {
            files,
            removed: Vec::new(),
            cursor: "folder".to_owned(),
        })
    }

    async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        fs::read(self.path(file_id)?).map_err(|e| io_error(e, file_id))
    }

    async fn upload(&self, area: Area, name: &str, data: Vec<u8>) -> Result<RemoteFile> {
        let id = format!("{}{name}", Self::prefix(area));
        let path = self.path(&id)?;
        let dir = self.dir(area);
        fs::create_dir_all(&dir).map_err(|e| io_error(e, &dir.display().to_string()))?;
        fs::write(&path, &data).map_err(|e| io_error(e, &id))?;
        let modified = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        Ok(RemoteFile {
            id,
            area,
            name: name.to_owned(),
            size: data.len() as u64,
            modified,
        })
    }

    async fn delete(&self, file_id: &str) -> Result<()> {
        fs::remove_file(self.path(file_id)?).map_err(|e| io_error(e, file_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn files_round_trip_and_paths_stay_inside() {
        let dir = tempfile::tempdir().unwrap();
        let p = FolderProvider::new(dir.path());
        assert!(p.list(Area::Objects).await.unwrap().is_empty());

        let f = p
            .upload(Area::Objects, "a.b.mmr", b"one".to_vec())
            .await
            .unwrap();
        assert_eq!(f.id, "objects/a.b.mmr");
        p.upload(Area::Root, "vault.json", b"{}".to_vec())
            .await
            .unwrap();
        assert_eq!(p.download(&f.id).await.unwrap(), b"one");
        let listed = p.list(Area::Objects).await.unwrap();
        assert_eq!((listed[0].name.as_str(), listed[0].size), ("a.b.mmr", 3));
        // The root listing has only files, not the area folders.
        let root: Vec<String> = p
            .list(Area::Root)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(root, ["vault.json"]);
        assert_eq!(p.changes("x").await.unwrap().files.len(), 2);

        for bad in ["../secret", "objects/../../x", "C:\\x", "/etc/passwd"] {
            assert!(
                matches!(p.download(bad).await, Err(ProviderError::NotFound(_))),
                "{bad}"
            );
        }
        p.delete(&f.id).await.unwrap();
        assert!(matches!(
            p.download(&f.id).await,
            Err(ProviderError::NotFound(_))
        ));
    }
}
