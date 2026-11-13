//! Embeds the icon and the version information in the executable as Windows resources.
//!
//! The `.res` file is written here, so no C preprocessor is needed. MSVC links it directly; the GNU
//! toolchain needs `windres` (from `WINDRES` or `PATH`) to turn it into an object. When that fails the
//! build continues with a warning, unless `CLUTCH_REQUIRE_RESOURCES` is set, as in the release workflow.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ICON: &str = "assets/icon.ico";
const PRODUCT: &str = "Clutch";
const INTERNAL_NAME: &str = "Clutch";
const ORIGINAL_FILENAME: &str = "Clutch.exe";
const LANGUAGE: u16 = 0x0409;
const CODE_PAGE: u16 = 1200;
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const MEMORY_FLAGS: u16 = 0x1030;

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=WINDRES");
    println!("cargo:rerun-if-env-changed=CLUTCH_REQUIRE_RESOURCES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match (embed(), env::var_os("CLUTCH_REQUIRE_RESOURCES").is_some()) {
        (Ok(()), _) => {}
        (Err(reason), true) => panic!("could not embed the Windows resources: {reason}"),
        (Err(reason), false) => println!("cargo:warning=built without the embedded icon and version info: {reason}"),
    }
}

fn embed() -> Result<(), String> {
    let out = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let icon = fs::read(ICON).map_err(|e| format!("{ICON}: {e}"))?;
    let res = out.join("resources.res");
    fs::write(&res, resources(&icon)?).map_err(|e| format!("{}: {e}", res.display()))?;
    let linkable = match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => res,
        _ => to_object(&res, &out.join("resources.o"))?,
    };
    println!("cargo:rustc-link-arg-bins={}", linkable.display());
    Ok(())
}

fn to_object(res: &Path, object: &Path) -> Result<PathBuf, String> {
    let windres = env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
    let output = Command::new(&windres)
        .args(["--input-format=res", "--output-format=coff", "--input"])
        .arg(res)
        .arg("--output")
        .arg(object)
        .output()
        .map_err(|e| format!("could not run {windres} ({e})"))?;
    match output.status.success() {
        true => Ok(object.to_path_buf()),
        false => Err(format!("{windres} failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
    }
}

struct Image<'a> {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits: u16,
    data: &'a [u8],
}

fn icon_images(icon: &[u8]) -> Result<Vec<Image<'_>>, String> {
    let invalid = || format!("{ICON} is not a valid .ico file");
    let u16_at = |at: usize| icon.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| icon.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    match (u16_at(0), u16_at(2), u16_at(4)) {
        (Some(0), Some(1), Some(count)) if count > 0 => (0..count as usize)
            .map(|index| {
                let at = 6 + index * 16;
                let size = u32_at(at + 8).ok_or_else(invalid)? as usize;
                let offset = u32_at(at + 12).ok_or_else(invalid)? as usize;
                Ok(Image {
                    width: icon[at],
                    height: icon[at + 1],
                    colors: icon[at + 2],
                    planes: u16_at(at + 4).ok_or_else(invalid)?,
                    bits: u16_at(at + 6).ok_or_else(invalid)?,
                    data: icon.get(offset..offset + size).ok_or_else(invalid)?,
                })
            })
            .collect(),
        _ => Err(invalid()),
    }
}

fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn dwords(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(|w| w.to_le_bytes()).collect()
}

fn pad(data: &mut Vec<u8>) {
    while data.len() % 4 != 0 {
        data.push(0);
    }
}

/// One entry of a `.res` file, identified by numeric type and name.
fn entry(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, language: u16, data: &[u8]) {
    res.extend(dwords(&[data.len() as u32, 32]));
    res.extend(words(&[0xFFFF, kind, 0xFFFF, name]));
    res.extend(dwords(&[0]));
    res.extend(words(&[flags, language]));
    res.extend(dwords(&[0, 0]));
    res.extend(data);
    pad(res);
}

fn resources(icon: &[u8]) -> Result<Vec<u8>, String> {
    let images = icon_images(icon)?;
    let mut res = Vec::new();
    entry(&mut res, 0, 0, 0, 0, &[]);
    for (index, image) in images.iter().enumerate() {
        entry(&mut res, RT_ICON, index as u16 + 1, MEMORY_FLAGS, LANGUAGE, image.data);
    }
    entry(&mut res, RT_GROUP_ICON, 1, MEMORY_FLAGS, LANGUAGE, &icon_group(&images));
    entry(&mut res, RT_VERSION, 1, MEMORY_FLAGS, LANGUAGE, &version_info());
    Ok(res)
}

fn icon_group(images: &[Image]) -> Vec<u8> {
    let mut data = words(&[0, 1, images.len() as u16]);
    for (index, image) in images.iter().enumerate() {
        data.extend([image.width, image.height, image.colors, 0]);
        data.extend(words(&[image.planes, image.bits]));
        data.extend(dwords(&[image.data.len() as u32]));
        data.extend(words(&[index as u16 + 1]));
    }
    data
}

enum Value<'a> {
    Empty,
    Text(&'a str),
    Binary(&'a [u8]),
}

/// A node of the version resource: length, value length, type, key, value and children, each 32-bit aligned.
fn block(key: &str, value: Value<'_>, children: Vec<Vec<u8>>) -> Vec<u8> {
    let (value_length, kind, bytes) = match value {
        Value::Empty => (0, 1u16, Vec::new()),
        Value::Text(text) => {
            let encoded = utf16z(text);
            (encoded.len() / 2, 1, encoded)
        }
        Value::Binary(data) => (data.len(), 0, data.to_vec()),
    };
    let mut data = vec![0u8; 6];
    data.extend(utf16z(key));
    pad(&mut data);
    data.extend(bytes);
    for child in children {
        pad(&mut data);
        data.extend(child);
    }
    let header = words(&[data.len() as u16, value_length as u16, kind]);
    data[..6].copy_from_slice(&header);
    data
}

fn version_info() -> Vec<u8> {
    let part = |name: &str| env::var(name).ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let most = (part("CARGO_PKG_VERSION_MAJOR") << 16) | part("CARGO_PKG_VERSION_MINOR");
    let least = part("CARGO_PKG_VERSION_PATCH") << 16;
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let fixed = dwords(&[0xFEEF_04BD, 0x0001_0000, most, least, most, least, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0]);
    let strings = [
        ("FileDescription", PRODUCT),
        ("FileVersion", version.as_str()),
        ("InternalName", INTERNAL_NAME),
        ("OriginalFilename", ORIGINAL_FILENAME),
        ("ProductName", PRODUCT),
        ("ProductVersion", version.as_str()),
    ];
    let table = block(
        &format!("{LANGUAGE:04X}{CODE_PAGE:04X}"),
        Value::Empty,
        strings.iter().map(|&(key, value)| block(key, Value::Text(value), Vec::new())).collect(),
    );
    let translation = words(&[LANGUAGE, CODE_PAGE]);
    block(
        "VS_VERSION_INFO",
        Value::Binary(&fixed),
        vec![
            block("StringFileInfo", Value::Empty, vec![table]),
            block("VarFileInfo", Value::Empty, vec![block("Translation", Value::Binary(&translation), Vec::new())]),
        ],
    )
}
