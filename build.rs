fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(target_os = "windows")]
    {
        // Deep GPUI widget composition in debug builds can exceed MSVC's 1MB default.
        println!("cargo:rustc-link-arg-bin=simple-snip=/STACK:8388608");
        println!("cargo:rerun-if-changed=packaging/windows/app.rc");
        println!("cargo:rerun-if-changed=assets/icons/app.ico");
        embed_resource::compile("packaging/windows/app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("Windows resource compiler is required to embed the application icon");
    }
}
