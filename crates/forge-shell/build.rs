//! Build script.
//!
//! The crate's real `Cargo.toml` lives at `crates/forge-shell/Cargo.toml`,
//! but Tauri-specific assets (`tauri.conf.json`, `capabilities/`, `icons/`)
//! live under `src-tauri/` per the task's design. `tauri-build` and
//! `tauri::generate_context!` both read `tauri.conf.json` from the current
//! working directory and require a `Cargo.toml` next to it, so this script:
//!
//! 1. Materializes placeholder icons so the bundle config's icon paths
//!    resolve (real branded icons land in a later task).
//! 2. Symlinks `src-tauri/Cargo.toml` to the crate's `Cargo.toml` so
//!    `tauri-build` can locate the manifest after we change into
//!    `src-tauri/`.
//! 3. Chdirs into `src-tauri/` and delegates to `tauri_build::build()`.

#[cfg(feature = "webview")]
fn main() {
    use std::path::PathBuf;

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let crate_root = PathBuf::from(&manifest_dir);
    let src_tauri = crate_root.join("src-tauri");
    let icon_dir = src_tauri.join("icons");
    std::fs::create_dir_all(&icon_dir).expect("create src-tauri/icons/");

    write_gray_png(&icon_dir.join("32x32.png"), 32);
    write_gray_png(&icon_dir.join("128x128.png"), 128);
    write_gray_png(&icon_dir.join("128x128@2x.png"), 256);
    // `.icns` and `.ico` are consumed by the bundler only. `bundle.active` is
    // false in the config, so empty files satisfy path resolution on Linux;
    // the real assets will be regenerated when bundling is turned on.
    touch(&icon_dir.join("icon.icns"));
    touch(&icon_dir.join("icon.ico"));

    ensure_cargo_toml_symlink(&crate_root, &src_tauri);

    std::env::set_current_dir(&src_tauri).expect("chdir to src-tauri/");
    tauri_build::build();
}

#[cfg(not(feature = "webview"))]
fn main() {}

#[cfg(feature = "webview")]
fn write_gray_png(path: &std::path::Path, size: u32) {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .unwrap_or_else(|e| panic!("png header: {e}"));
        let pixels: Vec<u8> = (0..size * size)
            .flat_map(|_| [0x50, 0x50, 0x50, 0xff])
            .collect();
        writer
            .write_image_data(&pixels)
            .unwrap_or_else(|e| panic!("png data: {e}"));
    }
    std::fs::write(path, buf).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

#[cfg(feature = "webview")]
fn touch(path: &std::path::Path) {
    if !path.exists() {
        std::fs::write(path, []).unwrap_or_else(|e| panic!("touch {}: {e}", path.display()));
    }
}

/// Ensure `src-tauri/Cargo.toml` points at the real crate manifest so
/// `tauri-build` can find it after this script chdirs into `src-tauri/`.
#[cfg(all(feature = "webview", unix))]
fn ensure_cargo_toml_symlink(crate_root: &std::path::Path, src_tauri: &std::path::Path) {
    let link = src_tauri.join("Cargo.toml");
    if link.exists() || link.symlink_metadata().is_ok() {
        return;
    }
    let target = crate_root.join("Cargo.toml");
    std::os::unix::fs::symlink(&target, &link)
        .unwrap_or_else(|e| panic!("symlink {} -> {}: {e}", link.display(), target.display()));
}

#[cfg(all(feature = "webview", windows))]
fn ensure_cargo_toml_symlink(crate_root: &std::path::Path, src_tauri: &std::path::Path) {
    // Fall back to copying on Windows where symlinks require elevated
    // permissions by default. Copy unconditionally so the mirror stays in
    // sync when the real manifest changes.
    let link = src_tauri.join("Cargo.toml");
    let target = crate_root.join("Cargo.toml");
    std::fs::copy(&target, &link)
        .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", target.display(), link.display()));
}
