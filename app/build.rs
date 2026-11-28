//! 把图标与版本信息嵌进 exe（只在 MSVC 目标上做事）。
//!
//! 用到 Windows SDK 的 rc.exe；找不到就跳过，构建仍然成功——只是 exe 没有图标。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=icon/zero.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let mut nums: Vec<u32> = version
        .split('.')
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .map(|p| p.parse::<u32>().unwrap_or(0))
        .collect();
    while nums.len() < 4 {
        nums.push(0);
    }
    let quad = format!("{},{},{},{}", nums[0], nums[1], nums[2], nums[3]);

    let icon = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("icon")
        .join("zero.ico");
    if !icon.exists() {
        println!("cargo:warning=icon/zero.ico 不存在，exe 不含图标");
        return;
    }

    let Some(rc_exe) = find_rc() else {
        println!("cargo:warning=没有找到 rc.exe（Windows SDK），exe 不含图标资源");
        return;
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc_path = out_dir.join("zero.rc");
    let res_path = out_dir.join("zero.res");

    let icon_path = icon.display().to_string().replace('\\', "\\\\");
    let body = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {quad}
PRODUCTVERSION {quad}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "highho"
            VALUE "FileDescription", "Zero - Smart Memory Workspace"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "zero-scanner"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "zero-scanner.exe"
            VALUE "ProductName", "Zero"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = icon_path,
        quad = quad,
        version = version
    );
    if std::fs::write(&rc_path, body).is_err() {
        println!("cargo:warning=写 .rc 失败，exe 不含图标资源");
        return;
    }

    let status = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status();
    match status {
        Ok(s) if s.success() && res_path.exists() => {
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
        _ => println!("cargo:warning=rc.exe 执行失败，exe 不含图标资源"),
    }
}

fn find_rc() -> Option<PathBuf> {
    // 1) PATH 里先找
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("rc.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // 2) Windows Kits，按版本号从高到低
    let mut kits: Vec<PathBuf> = Vec::new();
    for base in [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ] {
        let dir = Path::new(base);
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("x64").join("rc.exe");
            if candidate.exists() {
                kits.push(candidate);
            }
        }
    }
    kits.sort();
    kits.pop()
}
