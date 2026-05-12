use serde::Serialize;

const LOCAL_BACKUP_STATE_FILE_NAME: &str = "state.json";
const LOCAL_BACKUP_HISTORY_DIR_NAME: &str = "history";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalBackupStateResponse {
    pub(crate) document_json: Option<String>,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalBackupSyncResponse {
    pub(crate) synced_at: String,
    pub(crate) history_id: String,
    pub(crate) state_path: String,
    pub(crate) history_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalBackupHistoryEntry {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalBackupReadHistoryResponse {
    pub(crate) document_json: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalBackupRestoreResponse {
    pub(crate) synced_at: Option<String>,
}

fn normalize_backup_directory_path(directory_path: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = directory_path.trim();
    if trimmed.is_empty() {
        return Err("Backup directory path is empty".to_string());
    }

    let mut path = std::path::PathBuf::from(trimmed);
    if !path.is_absolute() {
        path = std::env::current_dir()
            .map_err(|err| format!("Failed to resolve current directory: {err}"))?
            .join(path);
    }

    Ok(path)
}

fn local_backup_state_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(LOCAL_BACKUP_STATE_FILE_NAME)
}

fn local_backup_history_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(LOCAL_BACKUP_HISTORY_DIR_NAME)
}

fn is_valid_local_backup_history_id(id: &str) -> bool {
    id.len() == 13 && id.chars().all(|char| char.is_ascii_digit())
}

fn local_backup_history_file_path(
    root: &std::path::Path,
    id: &str,
) -> Result<std::path::PathBuf, String> {
    if !is_valid_local_backup_history_id(id) {
        return Err("Invalid backup history identifier".to_string());
    }

    Ok(local_backup_history_dir(root).join(format!("{id}.json")))
}

fn ensure_local_backup_structure(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|err| {
        format!(
            "Failed creating backup directory '{}': {err}",
            root.display()
        )
    })?;

    std::fs::create_dir_all(local_backup_history_dir(root)).map_err(|err| {
        format!(
            "Failed creating backup history directory '{}': {err}",
            local_backup_history_dir(root).display()
        )
    })?;

    Ok(())
}

fn extract_backup_document_updated_at(raw: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;

    parsed
        .get("updatedAt")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            parsed
                .get("snapshot")
                .and_then(|snapshot| snapshot.get("updatedAt"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
}

#[tauri::command]
pub(crate) async fn local_backup_default_directory(
    app: crate::DragonFruitAppHandle,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;

        let base = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("Failed resolving app data directory: {err}"))?;

        let directory = base.join("backups");
        std::fs::create_dir_all(&directory).map_err(|err| {
            format!(
                "Failed creating default backup directory '{}': {err}",
                directory.display()
            )
        })?;

        Ok(directory.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Default backup directory task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_pick_directory(
    current_path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();

        if let Some(path) = current_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            dialog = dialog.set_directory(path);
        }

        let picked = dialog
            .pick_folder()
            .ok_or_else(|| "Folder selection cancelled by user".to_string())?;

        Ok(picked.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Pick backup folder task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_read_state(
    directory_path: String,
) -> Result<LocalBackupStateResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        let state_path = local_backup_state_path(&root);

        match std::fs::read_to_string(&state_path) {
            Ok(content) => Ok(LocalBackupStateResponse {
                updated_at: extract_backup_document_updated_at(&content),
                document_json: Some(content),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(LocalBackupStateResponse {
                    updated_at: None,
                    document_json: None,
                })
            }
            Err(error) => Err(format!(
                "Failed reading backup state file '{}': {error}",
                state_path.display()
            )),
        }
    })
    .await
    .map_err(|err| format!("Read backup state task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_sync(
    directory_path: String,
    document_json: String,
) -> Result<LocalBackupSyncResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        ensure_local_backup_structure(&root)?;

        let parsed = serde_json::from_str::<serde_json::Value>(&document_json)
            .map_err(|err| format!("Invalid backup document JSON: {err}"))?;
        let normalized_document = serde_json::to_string_pretty(&parsed)
            .map_err(|err| format!("Failed formatting backup document JSON: {err}"))?;

        let synced_at = parsed
            .get("updatedAt")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| {
                parsed
                    .get("snapshot")
                    .and_then(|snapshot| snapshot.get("updatedAt"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| String::new());

        let history_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string());

        let state_path = local_backup_state_path(&root);
        let history_path = local_backup_history_file_path(&root, &history_id)?;

        std::fs::write(&state_path, normalized_document.as_bytes()).map_err(|err| {
            format!(
                "Failed writing backup state file '{}': {err}",
                state_path.display()
            )
        })?;

        std::fs::write(&history_path, normalized_document.as_bytes()).map_err(|err| {
            format!(
                "Failed writing backup history file '{}': {err}",
                history_path.display()
            )
        })?;

        Ok(LocalBackupSyncResponse {
            synced_at,
            history_id,
            state_path: state_path.to_string_lossy().to_string(),
            history_path: history_path.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|err| format!("Local backup sync task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_list_history(
    directory_path: String,
) -> Result<Vec<LocalBackupHistoryEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        let history_dir = local_backup_history_dir(&root);

        if !history_dir.exists() {
            return Ok(Vec::new());
        }

        let read_dir = std::fs::read_dir(&history_dir).map_err(|err| {
            format!(
                "Failed listing backup history directory '{}': {err}",
                history_dir.display()
            )
        })?;

        let mut entries: Vec<LocalBackupHistoryEntry> = Vec::new();

        for dir_entry in read_dir.flatten() {
            let path = dir_entry.path();
            if !path.is_file() {
                continue;
            }

            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_default();
            if extension != "json" {
                continue;
            }

            let id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            if !is_valid_local_backup_history_id(&id) {
                continue;
            }

            let updated_at = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| extract_backup_document_updated_at(&content));

            entries.push(LocalBackupHistoryEntry {
                id,
                path: path.to_string_lossy().to_string(),
                updated_at,
            });
        }

        entries.sort_by(|left, right| right.id.cmp(&left.id));

        Ok(entries)
    })
    .await
    .map_err(|err| format!("List local backup history task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_read_history_item(
    directory_path: String,
    id: String,
) -> Result<LocalBackupReadHistoryResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        let history_path = local_backup_history_file_path(&root, id.trim())?;

        let document_json = std::fs::read_to_string(&history_path).map_err(|err| {
            format!(
                "Failed reading backup history file '{}': {err}",
                history_path.display()
            )
        })?;

        Ok(LocalBackupReadHistoryResponse { document_json })
    })
    .await
    .map_err(|err| format!("Read local backup history item task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_delete_history_item(
    directory_path: String,
    id: String,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        let history_path = local_backup_history_file_path(&root, id.trim())?;

        match std::fs::remove_file(&history_path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "Failed deleting backup history file '{}': {error}",
                history_path.display()
            )),
        }
    })
    .await
    .map_err(|err| format!("Delete local backup history item task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn local_backup_restore_history_item(
    directory_path: String,
    id: String,
) -> Result<LocalBackupRestoreResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = normalize_backup_directory_path(&directory_path)?;
        ensure_local_backup_structure(&root)?;

        let history_path = local_backup_history_file_path(&root, id.trim())?;
        let state_path = local_backup_state_path(&root);

        let document_json = std::fs::read_to_string(&history_path).map_err(|err| {
            format!(
                "Failed reading backup history file '{}': {err}",
                history_path.display()
            )
        })?;

        std::fs::write(&state_path, document_json.as_bytes()).map_err(|err| {
            format!(
                "Failed restoring backup state file '{}': {err}",
                state_path.display()
            )
        })?;

        Ok(LocalBackupRestoreResponse {
            synced_at: extract_backup_document_updated_at(&document_json),
        })
    })
    .await
    .map_err(|err| format!("Restore local backup history item task failed to join: {err}"))?
}
