fn main() {
    // `npm run build` fills it; an empty one lets a fresh clone compile.
    let _ = std::fs::create_dir_all("../frontend/dist");
    tauri_build::build()
}
