fn main() {
    let git_tag = std::process::Command::new("git")
        .args(["describe", "--tags", "--always"])
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

    #[cfg(all(target_os = "windows", feature = "winui-reactor-ui"))]
    {
        println!("cargo:rerun-if-changed=assets/provider-icons");
        copy_assets_to_target_dir();
        windows_reactor_setup::as_framework_dependent();
    }
}

#[cfg(all(target_os = "windows", feature = "winui-reactor-ui"))]
fn copy_assets_to_target_dir() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out_dir
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
    else {
        return;
    };

    let source = std::path::Path::new("assets").join("provider-icons");
    let target = profile_dir.join("Assets").join("provider-icons");
    if let Err(error) = copy_directory(&source, &target) {
        println!("cargo:warning=failed to copy WinUI assets: {error}");
    }
}

#[cfg(all(target_os = "windows", feature = "winui-reactor-ui"))]
fn copy_directory(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else {
            std::fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}
