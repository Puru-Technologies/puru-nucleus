fn main() {
    // On Windows, embed a manifest requesting admin elevation at launch.
    // This eliminates per-operation UAC prompts for installs, PATH changes, etc.
    #[cfg(target_os = "windows")]
    {
        let windows = tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("puru-nucleus.exe.manifest"));
        tauri_build::try_build(
            tauri_build::Attributes::new().windows_attributes(windows),
        )
        .expect("failed to run tauri build");
    }

    #[cfg(not(target_os = "windows"))]
    {
        tauri_build::build();
    }
}
