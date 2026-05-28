fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=resources/app.rc");
        println!("cargo:rerun-if-changed=assets/icons/quotify.ico");
        embed_resource::compile("resources/app.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
