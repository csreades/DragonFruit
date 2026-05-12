pub(crate) fn temp_artifact_path(extension: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let clean_ext = extension.trim_start_matches('.');
    path.push(format!("dragonfruit-slice-{stamp}.{clean_ext}"));
    path
}

pub(crate) fn is_dragonfruit_temp_artifact(path: &std::path::Path) -> bool {
    let file_name_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("dragonfruit-slice-"))
        .unwrap_or(false);
    let in_temp_dir = path.starts_with(std::env::temp_dir());
    file_name_ok && in_temp_dir
}

pub(crate) fn sweep_stale_temp_artifacts(max_age_seconds: u64) -> u32 {
    let mut removed = 0u32;
    let temp_dir = std::env::temp_dir();
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_seconds))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let entries = match std::fs::read_dir(&temp_dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_dragonfruit_temp_artifact(&path) {
            continue;
        }

        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|modified| modified <= cutoff)
            .unwrap_or(false);

        if stale && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}

pub(crate) fn sweep_all_temp_artifacts() -> u32 {
    let mut removed = 0u32;
    let temp_dir = std::env::temp_dir();
    let entries = match std::fs::read_dir(&temp_dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_dragonfruit_temp_artifact(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}
