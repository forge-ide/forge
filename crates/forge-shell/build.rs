//! Build script.
//!
//! Generates placeholder icons that `tauri.conf.json` references by path,
//! then delegates to `tauri_build::build()`. Cargo invokes build scripts
//! with CWD set to `CARGO_MANIFEST_DIR`, which is the crate root — the
//! same directory `tauri.conf.json` lives in — so no chdir or manifest
//! symlinking is needed.

#[cfg(feature = "webview")]
fn main() {
    use std::path::PathBuf;

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let crate_root = PathBuf::from(&manifest_dir);
    let icon_dir = crate_root.join("icons");
    std::fs::create_dir_all(&icon_dir).expect("create icons/");

    write_gray_png(&icon_dir.join("32x32.png"), 32);
    write_gray_png(&icon_dir.join("128x128.png"), 128);
    write_gray_png(&icon_dir.join("128x128@2x.png"), 256);
    // `.icns` and `.ico` are consumed by the bundler only. `bundle.active` is
    // false in the config, so empty files satisfy path resolution on Linux;
    // the real assets will be regenerated when bundling is turned on.
    touch(&icon_dir.join("icon.icns"));
    touch(&icon_dir.join("icon.ico"));

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
