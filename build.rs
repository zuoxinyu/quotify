fn main() {
    let git_tag = std::process::Command::new("git")
        .args(&["describe", "--tags", "--always"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=GIT_TAG={}", git_tag);

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=resources/app.rc");
        println!("cargo:rerun-if-changed=assets/icons/quotify.ico");
        embed_resource::compile("resources/app.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
