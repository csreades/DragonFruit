use serde::{Deserialize, Serialize};

const SCENE_AUTOSAVE_DIR_NAME: &str = "autosave";
const SCENE_AUTOSAVE_VOXL_FILE: &str = "scene.voxl";
const SCENE_AUTOSAVE_MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SceneAutosaveManifest {
    pub(crate) saved_at: String,
    pub(crate) clean: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SceneAutosavePaths {
    pub(crate) voxl_path: String,
    pub(crate) manifest_path: String,
}

fn scene_autosave_resolve_dir(
    app: &crate::DragonFruitAppHandle,
) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let base = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("Failed resolving app data dir: {err}"))?;
    let dir = base.join(SCENE_AUTOSAVE_DIR_NAME);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed creating autosave dir '{}': {err}", dir.display()))?;
    Ok(dir)
}

#[tauri::command]
pub(crate) async fn scene_autosave_get_paths(
    app: crate::DragonFruitAppHandle,
    preferred_save_path: Option<String>,
) -> Result<SceneAutosavePaths, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = scene_autosave_resolve_dir(&app)?;

        // Determine VOXL autosave path: if user has explicitly saved to a .voxl file,
        // autosave directly to that file. Otherwise use the generic recovery location.
        let voxl_path = if let Some(preferred) = preferred_save_path {
            let path = std::path::Path::new(&preferred);
            if crate::is_scene_file_path(path) && path.exists() {
                // User has explicitly saved; autosave directly to that file
                preferred
            } else {
                // Not a valid scene file or doesn't exist; fall back to generic recovery
                dir.join(SCENE_AUTOSAVE_VOXL_FILE)
                    .to_string_lossy()
                    .to_string()
            }
        } else {
            // No preferred path; use generic recovery location
            dir.join(SCENE_AUTOSAVE_VOXL_FILE)
                .to_string_lossy()
                .to_string()
        };

        Ok(SceneAutosavePaths {
            voxl_path,
            manifest_path: dir
                .join(SCENE_AUTOSAVE_MANIFEST_FILE)
                .to_string_lossy()
                .to_string(),
        })
    })
    .await
    .map_err(|err| format!("scene_autosave_get_paths task failed: {err}"))?
}

#[tauri::command]
pub(crate) async fn scene_autosave_write_manifest(
    app: crate::DragonFruitAppHandle,
    saved_at: String,
    clean: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = scene_autosave_resolve_dir(&app)?;
        let manifest = SceneAutosaveManifest { saved_at, clean };
        let json = serde_json::to_string(&manifest)
            .map_err(|err| format!("Failed serializing autosave manifest: {err}"))?;
        let path = dir.join(SCENE_AUTOSAVE_MANIFEST_FILE);
        std::fs::write(&path, json.as_bytes())
            .map_err(|err| format!("Failed writing autosave manifest: {err}"))?;
        Ok(())
    })
    .await
    .map_err(|err| format!("scene_autosave_write_manifest task failed: {err}"))?
}

#[tauri::command]
pub(crate) async fn scene_autosave_read_manifest(
    app: crate::DragonFruitAppHandle,
) -> Result<Option<SceneAutosaveManifest>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = scene_autosave_resolve_dir(&app)?;
        let path = dir.join(SCENE_AUTOSAVE_MANIFEST_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("Failed reading autosave manifest: {err}"))?;
        let manifest: SceneAutosaveManifest = serde_json::from_str(&content)
            .map_err(|err| format!("Failed parsing autosave manifest: {err}"))?;
        Ok(Some(manifest))
    })
    .await
    .map_err(|err| format!("scene_autosave_read_manifest task failed: {err}"))?
}

#[tauri::command]
pub(crate) async fn scene_autosave_read_voxl_bytes(
    app: crate::DragonFruitAppHandle,
) -> Result<tauri::ipc::Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let dir = scene_autosave_resolve_dir(&app)?;
        let path = dir.join(SCENE_AUTOSAVE_VOXL_FILE);
        if !path.exists() {
            return Err("No autosaved scene file found".to_string());
        }
        std::fs::read(&path).map_err(|err| format!("Failed reading autosaved scene: {err}"))
    })
    .await
    .map_err(|err| format!("scene_autosave_read_voxl_bytes task failed: {err}"))??;

    Ok(tauri::ipc::Response::new(bytes))
}
