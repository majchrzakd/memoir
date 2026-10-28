//! Workspaces: several notebooks on one device, e.g. a personal one in a
//! personal Google Drive and a work one in a work Drive. Each workspace is a
//! complete, separate notebook — its own vault (passphrase, recovery key),
//! encrypted database, attachments and Google sign-in — in its own folder.
//! One is open at a time; switching saves and pushes the open one first.
//!
//! The list lives in `workspaces.json` in the app's data folder (names only,
//! nothing secret). The first workspace uses the data folder itself, so a
//! notebook from before workspaces existed simply becomes it; the others
//! are in `workspaces/<id>/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use memoir_core::keystore::{SecretStore, cloud_account, local_db_account};
use memoir_core::store::ObjectId;
use memoir_core::vault::VaultFile;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::commands::AppState;
use crate::session::{Session, Status, write_atomically};

const LIST_FILE: &str = "workspaces.json";
const DIR: &str = "workspaces";
/// The workspace in the data folder itself.
const FIRST: &str = "default";
const FIRST_NAME: &str = "My notebook";
/// Longest a switch waits for the open workspace's changes to upload. The
/// rest stays queued and goes up when that workspace is open again.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ListFile {
    active: String,
    workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceList {
    pub active: String,
    pub workspaces: Vec<Workspace>,
}

pub struct Workspaces {
    root: PathBuf,
    keychain: Arc<dyn SecretStore>,
    list: std::sync::Mutex<ListFile>,
}

impl Workspaces {
    pub fn load(root: PathBuf, keychain: Arc<dyn SecretStore>) -> Self {
        let list = fs::read_to_string(root.join(LIST_FILE))
            .ok()
            .and_then(|json| serde_json::from_str::<ListFile>(&json).ok())
            .filter(|l| l.workspaces.iter().any(|w| w.id == l.active))
            .unwrap_or_else(|| ListFile {
                active: FIRST.into(),
                workspaces: vec![Workspace {
                    id: FIRST.into(),
                    name: FIRST_NAME.into(),
                }],
            });
        Self {
            root,
            keychain,
            list: std::sync::Mutex::new(list),
        }
    }

    /// Opens the active workspace's notebook.
    pub fn open_active(&self) -> Session {
        self.open(&self.active())
    }

    fn open(&self, id: &str) -> Session {
        Session::open(self.dir(id), Box::new(self.keychain.clone()))
    }

    pub fn list(&self) -> WorkspaceList {
        let list = self.list.lock().expect("lock");
        WorkspaceList {
            active: list.active.clone(),
            workspaces: list.workspaces.clone(),
        }
    }

    pub fn active(&self) -> String {
        self.list.lock().expect("lock").active.clone()
    }

    fn dir(&self, id: &str) -> PathBuf {
        if id == FIRST {
            self.root.clone()
        } else {
            self.root.join(DIR).join(id)
        }
    }

    fn update(&self, f: impl FnOnce(&mut ListFile) -> Result<(), String>) -> Result<(), String> {
        let mut list = self.list.lock().expect("lock");
        let mut changed = list.clone();
        f(&mut changed)?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&changed).expect("list serializes");
        write_atomically(&self.root.join(LIST_FILE), json.as_bytes())
            .map_err(|e| format!("could not save the workspace list: {e}"))?;
        *list = changed;
        Ok(())
    }

    fn add(&self, name: &str) -> Result<String, String> {
        let id = ObjectId::generate().to_string().to_lowercase();
        let name = clean_name(name)?;
        self.update(|list| {
            list.workspaces.push(Workspace {
                id: id.clone(),
                name,
            });
            Ok(())
        })?;
        Ok(id)
    }

    fn set_active(&self, id: &str) -> Result<(), String> {
        self.update(|list| {
            if !list.workspaces.iter().any(|w| w.id == id) {
                return Err("no such workspace".into());
            }
            list.active = id.to_owned();
            Ok(())
        })
    }

    fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let name = clean_name(name)?;
        self.update(|list| {
            let w = list
                .workspaces
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or("no such workspace")?;
            w.name = name;
            Ok(())
        })
    }

    /// Removes a workspace that isn't open, with its data on this device
    /// and its keys in the keychain. Its notes in Google Drive stay.
    fn remove(&self, id: &str) -> Result<(), String> {
        if id == self.active() {
            return Err("switch to another workspace first".into());
        }
        let dir = self.dir(id);
        self.update(|list| {
            list.workspaces.retain(|w| w.id != id);
            Ok(())
        })?;
        if let Some(file) = vault_in(&dir) {
            for account in [
                local_db_account(&file.vault_id),
                cloud_account(&file.vault_id),
            ] {
                if let Err(e) = self.keychain.remove(&account) {
                    log::warn!("could not remove a removed workspace's key: {e}");
                }
            }
        }
        if id == FIRST {
            // The data folder itself: only the notebook's files, not the
            // other workspaces, the list or the logs.
            for name in ["vault.json", "memoir.db", "memoir.db-wal", "memoir.db-shm"] {
                remove_if_exists(&dir.join(name), false)?;
            }
            for name in ["blobs", "opened"] {
                remove_if_exists(&dir.join(name), true)?;
            }
        } else {
            remove_if_exists(&dir, true)?;
        }
        log::info!("workspace removed from this device");
        Ok(())
    }

    /// The workspace (other than `except`) whose notebook is `vault_id`.
    pub fn holding(&self, vault_id: &str, except: &str) -> Option<Workspace> {
        let list = self.list.lock().expect("lock").workspaces.clone();
        list.into_iter().find(|w| {
            w.id != except && vault_in(&self.dir(&w.id)).is_some_and(|f| f.vault_id == vault_id)
        })
    }
}

fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.trim().chars().take(60).collect();
    if name.is_empty() {
        return Err("give the workspace a name".into());
    }
    Ok(name)
}

fn vault_in(dir: &Path) -> Option<VaultFile> {
    VaultFile::from_json(&fs::read_to_string(dir.join("vault.json")).ok()?).ok()
}

fn remove_if_exists(path: &Path, dir: bool) -> Result<(), String> {
    let result = if dir {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove the workspace's files: {e}")),
    }
}

/// Closes the open workspace and opens `id`. The UI saved the editor
/// first. Changes not uploaded yet are pushed (bounded); what doesn't make
/// it stays queued in that workspace until it's open again.
async fn switch<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    if tokio::time::timeout(PUSH_TIMEOUT, crate::sync::flush(app))
        .await
        .is_err()
    {
        log::warn!("switching workspace: upload timed out; the rest stays queued");
    }
    // No sync round may run while the notebook is replaced.
    let _round = state.sync.pause().await;
    let workspaces = app.state::<Workspaces>();
    workspaces.set_active(id)?;
    state.cloud.reset();
    {
        let mut session = state.session()?;
        session.lock();
        *session = workspaces.open(id);
        log::info!("switched workspace: {:?}", session.status());
    }
    crate::cloud::restore(&state);
    state.sync.reset(app);
    Ok(())
}

// ---- Commands -----------------------------------------------------------

#[tauri::command(async)]
pub fn workspaces(workspaces: State<'_, Workspaces>) -> WorkspaceList {
    workspaces.list()
}

#[tauri::command]
pub async fn switch_workspace<R: Runtime>(app: AppHandle<R>, id: String) -> Result<Status, String> {
    if id != app.state::<Workspaces>().active() {
        switch(&app, &id).await?;
    }
    Ok(app.state::<AppState>().session()?.status())
}

/// Adds a workspace and opens it; it starts at the first-run screen (a new
/// notebook, or one from Google Drive).
#[tauri::command]
pub async fn add_workspace<R: Runtime>(app: AppHandle<R>, name: String) -> Result<String, String> {
    let id = app.state::<Workspaces>().add(&name)?;
    switch(&app, &id).await?;
    Ok(id)
}

#[tauri::command(async)]
pub fn rename_workspace(
    workspaces: State<'_, Workspaces>,
    id: String,
    name: String,
) -> Result<(), String> {
    workspaces.rename(&id, &name)
}

#[tauri::command(async)]
pub fn remove_workspace(workspaces: State<'_, Workspaces>, id: String) -> Result<(), String> {
    workspaces.remove(&id)
}

#[cfg(test)]
mod tests {
    use memoir_core::keystore::MemoryStore;

    use super::*;

    fn workspaces(root: &Path) -> Workspaces {
        Workspaces::load(root.to_owned(), Arc::new(MemoryStore::default()))
    }

    #[test]
    fn the_first_workspace_is_the_data_folder() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces(root.path());
        let list = w.list();
        assert_eq!(list.active, FIRST);
        assert_eq!(list.workspaces.len(), 1);
        assert_eq!(w.dir(FIRST), root.path());
    }

    #[test]
    fn adding_renaming_switching_and_removing_are_remembered() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces(root.path());
        let work = w.add("  Work ").unwrap();
        assert!(w.add("   ").is_err());
        w.set_active(&work).unwrap();
        w.rename(FIRST, "Personal").unwrap();
        assert!(w.dir(&work).starts_with(root.path().join(DIR)));

        let w = workspaces(root.path());
        let list = w.list();
        assert_eq!(list.active, work);
        assert_eq!(
            list.workspaces
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>(),
            ["Personal", "Work"]
        );

        assert!(w.remove(&work).is_err(), "the open one can't be removed");
        fs::create_dir_all(w.dir(&work)).unwrap();
        fs::write(w.dir(&work).join("memoir.db"), b"x").unwrap();
        w.set_active(FIRST).unwrap();
        w.remove(&work).unwrap();
        assert!(!w.dir(&work).exists());
        assert_eq!(workspaces(root.path()).list().workspaces.len(), 1);
        assert!(w.set_active(&work).is_err());
    }

    #[test]
    fn removing_the_first_workspace_keeps_the_others() {
        let root = tempfile::tempdir().unwrap();
        let w = workspaces(root.path());
        let work = w.add("Work").unwrap();
        fs::create_dir_all(w.dir(&work)).unwrap();
        fs::write(root.path().join("memoir.db"), b"x").unwrap();
        fs::create_dir_all(root.path().join("blobs")).unwrap();
        w.set_active(&work).unwrap();
        w.remove(FIRST).unwrap();
        assert!(!root.path().join("memoir.db").exists());
        assert!(!root.path().join("blobs").exists());
        assert!(w.dir(&work).exists());
        assert!(root.path().join(LIST_FILE).exists());
    }
}
