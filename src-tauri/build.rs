fn main() {
    tauri_build::build();
    // The release embeds these assets, so frontend-only edits must rebuild it.
    println!("cargo:rerun-if-changed=../frontend");
}
