use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows.rc");
    println!("cargo:rerun-if-changed=packaging/netease-music-gpui.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("windows.rc");
    let version_numbers = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap())
        .join(",");
    let resource = include_str!("packaging/windows.rc")
        .replace("@VERSION_NUMBERS@", &format!("{version_numbers},0"))
        .replace("@VERSION@", &env::var("CARGO_PKG_VERSION").unwrap())
        .replace(
            "@ICON_PATH@",
            &root
                .join("packaging/netease-music-gpui.ico")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    fs::write(&output, resource).expect("write Windows application resources");
    // GPUI 按资源 ID 1 加载窗口图标；同一个图标也供资源管理器读取。
    embed_resource::compile_for(&output, ["netease-music-gpui"], embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows application icon and version information");
}
