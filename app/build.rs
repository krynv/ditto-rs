fn main() {
    // The web UI is embedded into the binary at compile time, but tauri-build only watches
    // tauri.conf.json and capabilities/: rebuild when the UI files change too.
    println!("cargo:rerun-if-changed=../ui");
    tauri_build::build()
}
