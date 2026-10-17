//! Windows: program icon and version information in rbx-ui.exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "rekordbox-rs")
            .set("FileDescription", "rekordbox-rs DJ player")
            .set("LegalCopyright", "MIT OR Apache-2.0");
        if let Err(e) = res.compile() {
            // no resource compiler: build without icon
            println!("cargo:warning=no Windows resources ({e})");
        }
    }
}
