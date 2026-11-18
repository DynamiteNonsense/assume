fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let is_windows = std::env::var("CARGO_CFG_TARGET_OS")
        .map(|v| v == "windows")
        .unwrap_or(false);
    if is_windows {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {}", e);
        }
    }
}
