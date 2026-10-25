fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=packaging/windows/app.rc");
        println!("cargo:rerun-if-changed=assets/icons/app.ico");
        embed_resource::compile("packaging/windows/app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("Windows resource compiler is required to embed the application icon");
    }
}
