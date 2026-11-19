use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=AUDIO_LOCK_REQUIRE_AUTHENTICODE");
    println!("cargo:rerun-if-env-changed=AUDIO_LOCK_EXPECTED_PUBLISHER");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let icon_path = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/tray must be nested two levels below workspace root")
        .join("assets")
        .join("app.ico");
    assert!(
        icon_path.is_file(),
        "application icon not found at {}",
        icon_path.display()
    );
    let icon_path_rc = icon_path
        .to_str()
        .expect("icon path must be valid UTF-8")
        .replace('\\', "\\\\");

    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set");
    let major: u16 = env::var("CARGO_PKG_VERSION_MAJOR")
        .expect("CARGO_PKG_VERSION_MAJOR must be set")
        .parse()
        .expect("CARGO_PKG_VERSION_MAJOR must be a valid u16");
    let minor: u16 = env::var("CARGO_PKG_VERSION_MINOR")
        .expect("CARGO_PKG_VERSION_MINOR must be set")
        .parse()
        .expect("CARGO_PKG_VERSION_MINOR must be a valid u16");
    let patch: u16 = env::var("CARGO_PKG_VERSION_PATCH")
        .expect("CARGO_PKG_VERSION_PATCH must be set")
        .parse()
        .expect("CARGO_PKG_VERSION_PATCH must be a valid u16");

    let rc = format!(
        r#"1 ICON "{icon_path_rc}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Tulantro LLC (Paragon Tweaks)\0"
            VALUE "FileDescription", "Audio Lock\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "default-audio-lock\0"
            VALUE "LegalCopyright", "@TheTulantro @ParagonTweaks\0"
            VALUE "OriginalFilename", "default-audio-lock.exe\0"
            VALUE "ProductName", "Audio Lock\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let rc_path = out_dir.join("version_info.rc");
    fs::write(&rc_path, rc).expect("failed to write version_info.rc");

    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile Windows VERSIONINFO resource");
}
