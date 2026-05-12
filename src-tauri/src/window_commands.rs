use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneFileHandoffPayload {
    paths: Vec<String>,
    source: String,
}

pub(crate) fn emit_scene_file_handoff(
    app: &crate::DragonFruitAppHandle,
    args: &[String],
    source: &str,
) {
    use tauri::Emitter;

    let paths = crate::scene_files::collect_scene_file_paths_from_args(args);
    if paths.is_empty() {
        return;
    }

    let payload = SceneFileHandoffPayload {
        paths,
        source: source.to_string(),
    };

    let _ = app.emit("dragonfruit://scene-file-handoff", payload);
}

pub(crate) fn focus_main_window(app: &crate::DragonFruitAppHandle) {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("main") {
        let is_visible = window.is_visible().unwrap_or(true);
        if !is_visible {
            let _ = window.show();
        }

        let is_minimized = window.is_minimized().unwrap_or(false);
        if is_minimized {
            let _ = window.unminimize();
        }

        let is_focused = window.is_focused().unwrap_or(false);
        if !is_focused {
            let _ = window.set_focus();
        }
    }
}

#[tauri::command]
pub(crate) fn get_slicer_engine_version() -> &'static str {
    dragonfruit_slicing_engine::ENGINE_VERSION
}

#[tauri::command]
pub(crate) async fn notify_launch_scene_handoff(
    app: crate::DragonFruitAppHandle,
) -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    emit_scene_file_handoff(&app, &args, "primary-launch");
    Ok(())
}

#[tauri::command]
pub(crate) async fn focus_main_window_command(
    app: crate::DragonFruitAppHandle,
) -> Result<(), String> {
    focus_main_window(&app);
    Ok(())
}

/// Reveals the main window without calling set_focus().
/// Used at startup to avoid triggering the Windows focus-stealing prevention
/// mechanism, which plays an error sound when SetForegroundWindow is called
/// from a process that does not currently own the foreground.
/// Also closes the splash screen window if it is still open.
#[tauri::command]
pub(crate) async fn reveal_main_window_command(
    app: crate::DragonFruitAppHandle,
) -> Result<(), String> {
    use tauri::Manager;

    // Show the main window first so there is no gap between splash close and
    // main window appearance (which would expose the desktop for a frame).
    // Maximize before show so the window is already at full size when it
    // becomes visible — avoids a two-step resize flash on Windows.
    if let Some(window) = app.get_webview_window("main") {
        let is_visible = window.is_visible().unwrap_or(true);
        if !is_visible {
            let _ = window.maximize();
            // Re-enable taskbar entry just before we make the window visible.
            #[cfg(target_os = "windows")]
            let _ = window.set_skip_taskbar(false);
            let _ = window.show();
        }
    }
    if let Some(splash) = app.get_webview_window("splashscreen") {
        let _ = splash.close();
    }
    Ok(())
}
