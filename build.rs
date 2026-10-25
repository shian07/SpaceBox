fn main() {
    // Embed the window icon + version metadata into the Windows .exe. Without
    // this the binary shows the default generic icon in Explorer and the taskbar.
    //
    // This must test the *target* OS, not the host: build scripts are compiled
    // for the host, so `#[cfg(windows)]` is false when cross-compiling to
    // Windows from Linux and the resources are silently skipped. Cargo exposes
    // the target as the CARGO_CFG_TARGET_OS environment variable.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/spacebox.ico")
            .set("ProductName", "Spacebox")
            .set("FileDescription", "Spacebox disk usage treemap")
            .set("LegalCopyright", "MIT License")
            .set("OriginalFilename", "spacebox.exe");
        if let Err(error) = res.compile() {
            // A missing resource compiler should not break the build; the app
            // still runs, it just gets the default icon.
            println!("cargo:warning=could not embed Windows resources: {error}");
        }
    }

    println!("cargo:rerun-if-changed=assets/spacebox.ico");
    println!("cargo:rerun-if-changed=assets/spacebox.png");
}
