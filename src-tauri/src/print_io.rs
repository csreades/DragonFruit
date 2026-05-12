use serde::{Deserialize, Serialize};
use tauri::ipc::Response;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePrintFileArgs {
    pub(crate) default_filename: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePrintFileFromPathArgs {
    pub(crate) default_filename: String,
    pub(crate) source_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickSavePathArgs {
    pub(crate) default_filename: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WriteBytesToPathArgs {
    pub(crate) destination_path: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickOpenFilesArgs {
    pub(crate) category: String,
    pub(crate) multiple: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickedOpenFile {
    pub(crate) path: String,
    pub(crate) name: String,
}

#[tauri::command]
pub(crate) async fn save_print_file(args: SavePrintFileArgs) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let suggested_name = {
            let trimmed = args.default_filename.trim();
            if trimmed.is_empty() {
                let format_provider = crate::plugin_registry::get_format_provider()
                    .unwrap_or_else(|_| crate::plugin_registry::get_default_format_provider());
                format_provider.default_export_filename()
            } else {
                trimmed.to_string()
            }
        };

        let picked = crate::build_save_dialog_with_filters(&suggested_name)
            .save_file()
            .ok_or_else(|| "Save cancelled by user".to_string())?;

        if let Some(parent) = picked.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed creating destination folder: {err}"))?;
        }

        std::fs::write(&picked, &args.bytes)
            .map_err(|err| format!("Failed saving print file: {err}"))?;

        Ok(picked.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Save task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn save_print_file_from_path(
    args: SavePrintFileFromPathArgs,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let suggested_name = {
            let trimmed = args.default_filename.trim();
            if trimmed.is_empty() {
                let format_provider = crate::plugin_registry::get_format_provider()
                    .unwrap_or_else(|_| crate::plugin_registry::get_default_format_provider());
                format_provider.default_export_filename()
            } else {
                trimmed.to_string()
            }
        };

        let source = std::path::PathBuf::from(args.source_path.trim());
        if !source.exists() {
            return Err("Source print file no longer exists on disk".to_string());
        }

        let picked = crate::build_save_dialog_with_filters(&suggested_name)
            .save_file()
            .ok_or_else(|| "Save cancelled by user".to_string())?;

        if let Some(parent) = picked.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed creating destination folder: {err}"))?;
        }

        std::fs::copy(&source, &picked)
            .map_err(|err| format!("Failed saving print file: {err}"))?;

        Ok(picked.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Save task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn pick_save_path(args: PickSavePathArgs) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let suggested_name = {
            let trimmed = args.default_filename.trim();
            if trimmed.is_empty() {
                let format_provider = crate::plugin_registry::get_format_provider()
                    .unwrap_or_else(|_| crate::plugin_registry::get_default_format_provider());
                format_provider.default_export_filename()
            } else {
                trimmed.to_string()
            }
        };

        let picked = crate::build_save_dialog_with_filters(&suggested_name)
            .save_file()
            .ok_or_else(|| "Save cancelled by user".to_string())?;

        Ok(picked.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Save picker task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn write_bytes_to_path(args: WriteBytesToPathArgs) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let trimmed_destination = args.destination_path.trim();
        if trimmed_destination.is_empty() {
            return Err("Destination path is empty".to_string());
        }

        let destination = std::path::PathBuf::from(trimmed_destination);

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed creating destination folder: {err}"))?;
        }

        std::fs::write(&destination, &args.bytes)
            .map_err(|err| format!("Failed writing file bytes: {err}"))?;

        Ok(destination.to_string_lossy().to_string())
    })
    .await
    .map_err(|err| format!("Write-bytes task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn pick_open_files(
    args: PickOpenFilesArgs,
) -> Result<Vec<PickedOpenFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = crate::build_open_dialog_with_filters(&args.category);

        let picked_paths: Vec<std::path::PathBuf> = if args.multiple {
            dialog.pick_files().unwrap_or_default()
        } else {
            match dialog.pick_file() {
                Some(path) => vec![path],
                None => Vec::new(),
            }
        };

        if picked_paths.is_empty() {
            return Err("Open cancelled by user".to_string());
        }

        let files = picked_paths
            .into_iter()
            .map(|path| PickedOpenFile {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
                    .to_string(),
                path: path.to_string_lossy().to_string(),
            })
            .collect::<Vec<_>>();

        Ok(files)
    })
    .await
    .map_err(|err| format!("Open picker task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn get_launch_scene_files() -> Result<Vec<PickedOpenFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files =
            crate::collect_scene_file_paths_from_args(&std::env::args().collect::<Vec<_>>())
                .into_iter()
                .map(|path_text| {
                    let path = std::path::PathBuf::from(&path_text);
                    PickedOpenFile {
                        name: path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("file")
                            .to_string(),
                        path: path_text,
                    }
                })
                .collect::<Vec<_>>();

        Ok(files)
    })
    .await
    .map_err(|err| format!("Launch scene-files task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn read_print_file_bytes(source_path: String) -> Result<Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let source = std::path::PathBuf::from(source_path.trim());
        if !source.exists() {
            return Err("Source print file no longer exists on disk".to_string());
        }

        std::fs::read(&source).map_err(|err| format!("Failed reading print file: {err}"))
    })
    .await
    .map_err(|err| format!("Read task failed to join: {err}"))??;

    Ok(Response::new(bytes))
}

#[tauri::command]
pub(crate) async fn read_print_file_size(source_path: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = std::path::PathBuf::from(source_path.trim());
        if !source.exists() {
            return Err("Source print file no longer exists on disk".to_string());
        }

        std::fs::metadata(&source)
            .map(|meta| meta.len())
            .map_err(|err| format!("Failed reading print file metadata: {err}"))
    })
    .await
    .map_err(|err| format!("Read-size task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn read_print_file_chunk(
    source_path: String,
    offset: u64,
    length: u64,
) -> Result<Response, String> {
    const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let source = std::path::PathBuf::from(source_path.trim());
        if !source.exists() {
            return Err("Source print file no longer exists on disk".to_string());
        }

        let mut file = std::fs::File::open(&source)
            .map_err(|err| format!("Failed opening print file: {err}"))?;

        let file_len = file
            .metadata()
            .map_err(|err| format!("Failed reading print file metadata: {err}"))?
            .len();

        if offset >= file_len {
            return Ok(Vec::new());
        }

        let remaining = file_len - offset;
        let requested = length.max(1).min(MAX_CHUNK_BYTES as u64).min(remaining) as usize;

        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))
            .map_err(|err| format!("Failed seeking print file chunk: {err}"))?;

        let mut chunk = vec![0u8; requested];
        file.read_exact(&mut chunk)
            .map_err(|err| format!("Failed reading print file chunk: {err}"))?;

        Ok(chunk)
    })
    .await
    .map_err(|err| format!("Read-chunk task failed to join: {err}"))??;

    Ok(Response::new(bytes))
}

#[tauri::command]
pub(crate) async fn read_print_layer_png(
    source_path: String,
    layer_number: u32,
    format_hint: String,
) -> Result<Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        if layer_number == 0 {
            return Err("Layer number must be >= 1".to_string());
        }

        let source = std::path::PathBuf::from(source_path.trim());
        if !source.exists() {
            return Err("Source print file no longer exists on disk".to_string());
        }

        dragonfruit_slicing_engine::engine::read_layer_preview_png_by_format_hint(
            &source,
            layer_number,
            &format_hint,
        )
        .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| format!("Read layer task failed to join: {err}"))??;

    Ok(Response::new(bytes))
}

#[tauri::command]
pub(crate) async fn delete_print_temp_file(source_path: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = std::path::PathBuf::from(source_path.trim());
        if !source.exists() {
            return Ok(false);
        }
        if !crate::is_dragonfruit_temp_artifact(&source) {
            return Err("Refusing to delete non-DragonFruit temp artifact path".to_string());
        }
        std::fs::remove_file(&source)
            .map_err(|err| format!("Failed deleting temp artifact: {err}"))?;
        Ok(true)
    })
    .await
    .map_err(|err| format!("Delete task failed to join: {err}"))?
}

#[tauri::command]
pub(crate) async fn cleanup_stale_print_temp_files(max_age_seconds: u64) -> Result<u32, String> {
    let age = max_age_seconds.max(60);
    let removed =
        tauri::async_runtime::spawn_blocking(move || crate::sweep_stale_temp_artifacts(age))
            .await
            .map_err(|err| format!("Cleanup task failed to join: {err}"))?;
    Ok(removed)
}

#[tauri::command]
pub(crate) async fn cleanup_all_print_temp_files() -> Result<u32, String> {
    let removed = tauri::async_runtime::spawn_blocking(crate::sweep_all_temp_artifacts)
        .await
        .map_err(|err| format!("Cleanup-all task failed to join: {err}"))?;
    Ok(removed)
}

/// Open the folder containing the given path in the OS file manager.
/// On Windows this uses `explorer /select,<path>` to highlight the file.
/// On macOS it uses `open -R <path>`. On Linux it falls back to `xdg-open`
/// on the parent directory.
#[tauri::command]
pub(crate) async fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("Path is empty".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| format!("Failed to open Explorer: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| format!("Failed to reveal in Finder: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        let p = std::path::Path::new(&path);
        let dir = p.parent().unwrap_or(p);
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {e}"))?;
    }

    Ok(())
}
