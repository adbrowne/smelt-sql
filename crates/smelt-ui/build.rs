use std::path::Path;

fn main() {
    // Ensure ui/dist exists so rust-embed's derive macro doesn't fail.
    // The directory may be empty (no frontend build), which is fine —
    // the server shows a helpful fallback message in that case.
    let dist_dir = Path::new("../../ui/dist");
    if !dist_dir.exists() {
        std::fs::create_dir_all(dist_dir).expect("failed to create ui/dist");
    }
}
