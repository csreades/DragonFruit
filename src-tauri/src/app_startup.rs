pub(crate) fn run() {
    // Issue #83: on Linux with Nvidia+Wayland, WebKitGTK's DMA-BUF renderer
    // can crash inside gbm_bo_create_with_modifiers. Setting this env var
    // before any GTK/WebKit init forces the SHM fallback path, which is
    // slightly slower but stable. Only applies to the wry (WebKitGTK) path;
    // CEF uses its own compositor and is unaffected.
    #[cfg(all(target_os = "linux", not(feature = "tauri-cef")))]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            // NOTE: called at the very start of main, single-threaded, before
            // any other threads or libraries are initialized. On Rust edition
            // 2024+ this will require an unsafe block (set_var became unsafe).
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    // Install a panic hook that writes the panic location and message to the
    // DragonFruit log before the default handler runs.  This ensures that even
    // hard crashes leave a human-readable trace in the log file rather than
    // only a terse Windows Event 1005 entry.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message: String = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(non-string panic payload)".to_string()
        };
        log::error!("[panic] {message} at {location}");
        default_panic_hook(info);
    }));

    // Sweep week-old stale temp artifacts on app startup.
    let _ = crate::temp_artifacts::sweep_stale_temp_artifacts(7 * 24 * 60 * 60);

    // Initialize plugin registry and register built-in plugins
    if let Err(error) = crate::plugin_registry::initialize_plugins() {
        if cfg!(debug_assertions) {
            log::warn!("[plugin-registry] WARNING: {error}");
        } else {
            panic!("Failed to initialize plugin registry: {error}");
        }
    }

    log::info!(
        "DragonFruit {} starting (debug={})",
        env!("CARGO_PKG_VERSION"),
        cfg!(debug_assertions)
    );

    let _log_level = crate::logging::read_log_level_pref();
    // Log plugin disabled on CEF — it pulls tauri/wry transitively, causing
    // E0252 collision. See Cargo.toml comment.
    #[cfg(not(feature = "tauri-cef"))]
    let log_plugin = {
        use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};
        LogBuilder::new()
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir {
                    file_name: Some("dragonfruit".to_string()),
                }),
                Target::new(TargetKind::Webview),
            ])
            .level(_log_level)
            // Suppress chatty low-level transport crates — these flood the log
            // at DEBUG/TRACE and contain no actionable application information.
            .level_for("tungstenite", log::LevelFilter::Warn)
            .level_for("reqwest", log::LevelFilter::Warn)
            .level_for("hyper", log::LevelFilter::Warn)
            .level_for("hyper_util", log::LevelFilter::Warn)
            .level_for("rustls", log::LevelFilter::Warn)
            .level_for("h2", log::LevelFilter::Warn)
            .level_for("tokio_tungstenite", log::LevelFilter::Warn)
            .max_file_size(5_000_000)
            .rotation_strategy(RotationStrategy::KeepOne)
            .build()
    };

    let builder = tauri::Builder::default();
    #[cfg(not(feature = "tauri-cef"))]
    let builder = builder.plugin(log_plugin);
    let builder = builder.setup(|app| {
        let app_handle = app.handle().clone();

        // Defer main window creation to an async task so the splashscreen's
        // WebView2 instance fully initialises before the main window's does.
        // On Windows, simultaneous WebView2 init produces a brief window flash
        // even when the main window is created with visible=false.
        tauri::async_runtime::spawn(async move {
            use tauri::WebviewWindowBuilder;

            // Give the splash WebView2 a head-start on Windows only.
            #[cfg(target_os = "windows")]
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;

            let config = app_handle.config();
            let Some(window_config) = config.app.windows.iter().find(|w| w.label == "main") else {
                log::error!("Missing 'main' window config in tauri.conf.json");
                return;
            };

            let builder = match WebviewWindowBuilder::from_config(&app_handle, window_config) {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to create main window builder: {e}");
                    return;
                }
            };

            // Keep custom titlebar behavior on non-macOS.
            #[cfg(not(target_os = "macos"))]
            let builder = builder.decorations(false);

            // Belt-and-suspenders: enforce hidden + no taskbar entry while
            // the window is invisible, regardless of config values. On Windows
            // a taskbar button flash is the tell-tale sign the window was
            // briefly visible during WebView2 initialisation.
            let builder = builder.visible(false);
            #[cfg(target_os = "windows")]
            let builder = builder.skip_taskbar(true);

            match builder.build() {
                Ok(_window) => {
                    // On macOS, reveal immediately so a frontend startup hiccup
                    // can't leave the app invisible when created as hidden.
                    #[cfg(target_os = "macos")]
                    {
                        if let Err(error) = _window.show() {
                            log::warn!("Failed to show main window during setup: {error}");
                        }
                    }
                    log::info!("Main window created successfully");
                }
                Err(e) => log::error!("Failed to build main window: {e}"),
            }
        });

        Ok(())
    });

    // Single-instance plugin also disabled on CEF (same wry collision).
    #[cfg(not(feature = "tauri-cef"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        log::info!("Single-instance activated: second launch detected (args={argv:?})");
        let has_scene_files = !crate::scene_files::collect_scene_file_paths_from_args(&argv).is_empty();
        crate::window_commands::emit_scene_file_handoff(app, &argv, "single-instance");

        // Only force foreground when this second launch is handing off a scene file.
        // Avoiding unconditional focus here reduces Windows "error" chimes when users
        // launch the app again while it's already running.
        if has_scene_files {
            crate::window_commands::focus_main_window(app);
        }
    }));

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_plugin_macos_fps::init());

    builder
        .invoke_handler(tauri::generate_handler![
            crate::slicing_staging::slice_solid_native,
            crate::slicing_staging::stage_mesh_binary_start,
            crate::slicing_staging::allocate_mesh_stage_path,
            crate::slicing_staging::append_mesh_stage_chunk,
            crate::slicing_staging::finish_mesh_stage_write,
            crate::slicing_staging::stage_mesh_file_path,
            crate::slicing_staging::stage_mesh_binary_set,
            crate::slicing_staging::stage_mesh_binary_chunk,
            crate::slicing_staging::slice_solid_native_to_temp_path,
            crate::slicing_staging::cancel_slicing,
            crate::island_scan::run_island_scan_native,
            crate::mesh_export::export_mesh_file,
            crate::print_io::save_print_file,
            crate::print_io::save_print_file_from_path,
            crate::print_io::pick_save_path,
            crate::print_io::pick_open_files,
            crate::print_io::get_launch_scene_files,
            crate::window_commands::get_slicer_engine_version,
            crate::window_commands::notify_launch_scene_handoff,
            crate::window_commands::focus_main_window_command,
            crate::window_commands::reveal_main_window_command,
            crate::print_io::write_bytes_to_path,
            crate::print_io::read_print_file_bytes,
            crate::print_io::read_print_file_size,
            crate::print_io::read_print_file_chunk,
            crate::print_io::read_print_layer_png,
            crate::print_io::delete_print_temp_file,
            crate::print_io::cleanup_stale_print_temp_files,
            crate::print_io::cleanup_all_print_temp_files,
            crate::local_backup::local_backup_default_directory,
            crate::local_backup::local_backup_pick_directory,
            crate::local_backup::local_backup_read_state,
            crate::local_backup::local_backup_sync,
            crate::local_backup::local_backup_list_history,
            crate::local_backup::local_backup_read_history_item,
            crate::local_backup::local_backup_delete_history_item,
            crate::local_backup::local_backup_restore_history_item,
            crate::scene_autosave::scene_autosave_get_paths,
            crate::scene_autosave::scene_autosave_write_manifest,
            crate::scene_autosave::scene_autosave_read_manifest,
            crate::scene_autosave::scene_autosave_read_voxl_bytes,
            crate::print_io::reveal_in_file_manager,
            crate::logging::set_log_level_pref,
            crate::logging::read_log_tail,
            crate::logging::open_log_file,
            crate::logging::delete_log_file,
            crate::network::plugin_network_request,
            crate::network::ensure_rtsp_relay,
            crate::mesh_repair::mesh_analyze_from_path,
            crate::mesh_repair::mesh_analyze_staged,
            crate::mesh_repair::mesh_repair_from_path,
            crate::mesh_repair::mesh_repair_staged,
            crate::mesh_repair::mesh_classify_staged,
            crate::mesh_repair::mesh_repair_read_positions
        ])
        .run(tauri::generate_context!())
        .expect("error while running DragonFruit desktop app");
}
