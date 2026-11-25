// Windows: embeds the file icon, version info and application manifest into
// planar.exe. Windows Explorer, SmartScreen and antivirus scanners read these;
// an executable without them looks suspicious.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/planar.ico");
    println!("cargo:rerun-if-changed=assets/planar.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/planar.ico")
        .set_manifest_file("assets/planar.manifest")
        .set("ProductName", "Planar")
        .set("FileDescription", "Planar")
        .set("CompanyName", "Michael Blachetta")
        .set("LegalCopyright", "Copyright (c) 2026 Michael Blachetta. MIT License.")
        .set("OriginalFilename", "planar.exe")
        .set("InternalName", "planar");
    res.compile().expect("failed to embed the Windows resources (icon, version info, manifest)");
}
