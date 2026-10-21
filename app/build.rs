//! Embeds the RustyCorn icon and version information in the Windows executable
//! (Explorer, taskbar pinning, shortcuts, Properties → Details). Linux and
//! macOS executables carry no icon; there the window icon set at start is used.
//!
//! Cross-building from Linux needs `x86_64-w64-mingw32-windres` (mingw-w64).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/rustycorn.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/rustycorn.ico");
        // FileVersion and ProductVersion come from the package version.
        res.set("ProductName", "RustyCorn");
        res.set("FileDescription", "RustyCorn — ÄKTAprime logger");
        res.set("OriginalFilename", "RustyCorn.exe");
        res.set("InternalName", "rustycorn");
        res.set(
            "LegalCopyright",
            "MIT licence. Copyright (c) 2026 Calvin Klein",
        );
        if let Err(err) = res.compile() {
            println!("cargo:warning=Could not embed the Windows icon: {err}");
        }
    }
}
