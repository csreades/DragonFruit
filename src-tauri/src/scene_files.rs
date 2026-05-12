pub(crate) fn is_scene_file_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let normalized = ext.trim().trim_start_matches('.');
            normalized.eq_ignore_ascii_case("voxl")
                || crate::BUILTIN_PLUGIN_SCENE_EXTENSIONS
                    .iter()
                    .any(|&s| normalized.eq_ignore_ascii_case(s))
        })
        .unwrap_or(false)
}

pub(crate) fn collect_scene_file_paths_from_args(args: &[String]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            continue;
        }

        let path = std::path::PathBuf::from(trimmed);
        if !path.is_file() {
            continue;
        }

        if !is_scene_file_path(&path) {
            continue;
        }

        files.push(path.to_string_lossy().to_string());
    }

    files
}

pub(crate) fn build_open_dialog_with_filters(category: &str) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new();

    // Build scene extension list: voxl + all plugin-registered extensions + zip (for bundles)
    let mut scene_exts: Vec<&str> = vec!["voxl"];
    scene_exts.extend_from_slice(crate::BUILTIN_PLUGIN_SCENE_EXTENSIONS);
    scene_exts.push("zip");

    let normalized = category.trim().to_ascii_lowercase();
    dialog = match normalized.as_str() {
        "mesh" => dialog.add_filter("Mesh Files", &["stl", "obj", "3mf", "zip"]),
        "scene" => dialog.add_filter("Scene Files", &scene_exts),
        "bundle" => dialog.add_filter("JSON Files", &["json"]),
        _ => dialog
            .add_filter("Mesh Files", &["stl", "obj", "3mf", "zip"])
            .add_filter("Scene Files", &scene_exts),
    };

    dialog
}
