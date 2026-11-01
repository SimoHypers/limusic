//! Pinned ONNX Runtime package metadata, archive extraction, and verification.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const VERSION: &str = "1.24.4";

pub const GITHUB_HOSTS: &[&str] =
    &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Tgz,
    Zip,
    ZipRange,
}

#[derive(Debug, Clone)]
pub struct PartFile {
    pub entry: &'static str,
    pub dest: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct Part {
    pub name: &'static str,
    pub url: &'static str,
    pub archive_size: u64,
    pub archive_sha256: &'static str,
    pub fetch_size: u64,
    pub kind: ArchiveKind,
    pub hosts: &'static [&'static str],
    pub files: &'static [PartFile],
}

#[derive(Debug, Clone)]
pub struct RuntimePackage {
    pub platform: &'static str,
    pub parts: &'static [Part],
}

pub static LINUX_X86_64: RuntimePackage = RuntimePackage {
    platform: "linux-x86_64",
    parts: &[Part {
        name: "onnxruntime",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-linux-x64-1.24.4.tgz",
        archive_size: 8155822,
        archive_sha256: "3a211fbea252c1e66290658f1b735b772056149f28321e71c308942cdb54b747",
        fetch_size: 8155822,
        kind: ArchiveKind::Tgz,
        hosts: GITHUB_HOSTS,
        files: &[
            PartFile {
                entry: "onnxruntime-linux-x64-1.24.4/lib/libonnxruntime.so.1.24.4",
                dest: "libonnxruntime.so.1.24.4",
                size: 22159232,
                sha256: "d132535d051344ff5c64c9c200004150559049a81ed330eb4422c1962fb6b7e4",
                optional: false,
            },
            PartFile {
                entry: "onnxruntime-linux-x64-1.24.4/LICENSE",
                dest: "LICENSE",
                size: 0,
                sha256: "",
                optional: true,
            },
            PartFile {
                entry: "onnxruntime-linux-x64-1.24.4/ThirdPartyNotices.txt",
                dest: "ThirdPartyNotices.txt",
                size: 0,
                sha256: "",
                optional: true,
            },
            PartFile {
                entry: "onnxruntime-linux-x64-1.24.4/Privacy.md",
                dest: "Privacy.md",
                size: 0,
                sha256: "",
                optional: true,
            },
        ],
    }],
};

pub static MACOS_AARCH64: RuntimePackage = RuntimePackage {
    platform: "macos-aarch64",
    parts: &[Part {
        name: "onnxruntime",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-osx-arm64-1.24.4.tgz",
        archive_size: 30937282,
        archive_sha256: "93787795f47e1eee369182e43ed51b9e5da0878ab0346aecf4258979b8bba989",
        fetch_size: 30937282,
        kind: ArchiveKind::Tgz,
        hosts: GITHUB_HOSTS,
        files: &[
            PartFile {
                entry: "onnxruntime-osx-arm64-1.24.4/lib/libonnxruntime.1.24.4.dylib",
                dest: "libonnxruntime.1.24.4.dylib",
                size: 35418600,
                sha256: "872533f130f1839a5bc01788ddb4f75c83a189763441ba1178788ed965449289",
                optional: false,
            },
            PartFile {
                entry: "onnxruntime-osx-arm64-1.24.4/LICENSE",
                dest: "LICENSE",
                size: 0,
                sha256: "",
                optional: true,
            },
            PartFile {
                entry: "onnxruntime-osx-arm64-1.24.4/ThirdPartyNotices.txt",
                dest: "ThirdPartyNotices.txt",
                size: 0,
                sha256: "",
                optional: true,
            },
            PartFile {
                entry: "onnxruntime-osx-arm64-1.24.4/Privacy.md",
                dest: "Privacy.md",
                size: 0,
                sha256: "",
                optional: true,
            },
        ],
    }],
};

pub const NUGET_HOSTS: &[&str] = &["api.nuget.org"];

/// Windows: onnxruntime.dll from the official DirectML NuGet package (whole package, 12.5 MB) and DirectML.dll from the 202 MB Microsoft.AI.DirectML package, of which only the one file is fetched with Range requests.
pub static WINDOWS_X86_64: RuntimePackage = RuntimePackage {
    platform: "windows-x86_64",
    parts: &[
        Part {
            name: "onnxruntime",
            url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg",
            archive_size: 12458649,
            archive_sha256: "57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681",
            fetch_size: 12458649,
            kind: ArchiveKind::Zip,
            hosts: NUGET_HOSTS,
            files: &[
                PartFile {
                    entry: "runtimes/win-x64/native/onnxruntime.dll",
                    dest: "onnxruntime.dll",
                    size: 17328152,
                    sha256: "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40",
                    optional: false,
                },
                PartFile {
                    entry: "LICENSE",
                    dest: "licenses/onnxruntime-LICENSE.txt",
                    size: 0,
                    sha256: "",
                    optional: true,
                },
                PartFile {
                    entry: "ThirdPartyNotices.txt",
                    dest: "licenses/onnxruntime-ThirdPartyNotices.txt",
                    size: 0,
                    sha256: "",
                    optional: true,
                },
            ],
        },
        Part {
            name: "directml",
            url: "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg",
            archive_size: 202292617,
            archive_sha256: "",
            fetch_size: 9420000,
            kind: ArchiveKind::ZipRange,
            hosts: NUGET_HOSTS,
            files: &[
                PartFile {
                    entry: "bin/x64-win/DirectML.dll",
                    dest: "DirectML.dll",
                    size: 18527776,
                    sha256: "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1",
                    optional: false,
                },
                PartFile {
                    entry: "LICENSE.txt",
                    dest: "licenses/DirectML-LICENSE.txt",
                    size: 0,
                    sha256: "",
                    optional: true,
                },
                PartFile {
                    entry: "LICENSE-CODE.txt",
                    dest: "licenses/DirectML-LICENSE-CODE.txt",
                    size: 0,
                    sha256: "",
                    optional: true,
                },
                PartFile {
                    entry: "ThirdPartyNotices.txt",
                    dest: "licenses/DirectML-ThirdPartyNotices.txt",
                    size: 0,
                    sha256: "",
                    optional: true,
                },
            ],
        },
    ],
};

/// The package for this build target, None on other targets.
pub fn current() -> Option<&'static RuntimePackage> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(&LINUX_X86_64)
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(&MACOS_AARCH64)
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some(&WINDOWS_X86_64)
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        None
    }
}

/// Folder name under the app's data dir where the runtime lives: format!("onnxruntime-{VERSION}")
pub fn dir_name() -> String {
    format!("onnxruntime-{VERSION}")
}

pub fn main_lib(pkg: &RuntimePackage) -> &PartFile {
    pinned_files(pkg).next().expect("package must have a main lib")
}

pub fn pinned_files(pkg: &RuntimePackage) -> impl Iterator<Item = &PartFile> {
    pkg.parts.iter().flat_map(|p| p.files.iter()).filter(|f| !f.sha256.is_empty() && !f.optional)
}

pub fn total_archive_size(pkg: &RuntimePackage) -> u64 {
    pkg.parts.iter().map(|p| p.fetch_size).sum()
}

pub const REMOTE_ZIP_TAIL: u64 = 65536;

pub fn extract_remote_zip(
    part: &Part,
    total_len: u64,
    tail: &[u8],
    dest_dir: &Path,
    fetch: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, crate::runtime::Unavailable>,
) -> Result<(), crate::runtime::Unavailable> {
    if total_len != part.archive_size {
        return Err(crate::runtime::Unavailable::Corrupt(part.name.into()));
    }

    let mut eocd_pos = None;
    if tail.len() >= 22 {
        for i in (0..=tail.len() - 22).rev() {
            if i + 4 <= tail.len() {
                let sig = u32::from_le_bytes(tail[i..i + 4].try_into().map_err(|_| {
                    crate::runtime::Unavailable::LoadFailed("zip: invalid data".into())
                })?);
                if sig == 0x06054b50 && i + 22 <= tail.len() {
                    let comment_len =
                        u16::from_le_bytes(tail[i + 20..i + 22].try_into().map_err(|_| {
                            crate::runtime::Unavailable::LoadFailed("zip: invalid data".into())
                        })?) as usize;
                    if i + 22 + comment_len <= tail.len() {
                        eocd_pos = Some(i);
                        break;
                    }
                }
            }
        }
    }
    let i = eocd_pos
        .ok_or_else(|| crate::runtime::Unavailable::LoadFailed("zip: no end record".into()))?;

    if i + 20 > tail.len() {
        return Err(crate::runtime::Unavailable::LoadFailed("zip: invalid end record".into()));
    }
    let entry_count = u16::from_le_bytes(
        tail[i + 10..i + 12]
            .try_into()
            .map_err(|_| crate::runtime::Unavailable::LoadFailed("zip: invalid data".into()))?,
    );
    let cd_size = u32::from_le_bytes(
        tail[i + 12..i + 16]
            .try_into()
            .map_err(|_| crate::runtime::Unavailable::LoadFailed("zip: invalid data".into()))?,
    );
    let cd_offset = u32::from_le_bytes(
        tail[i + 16..i + 20]
            .try_into()
            .map_err(|_| crate::runtime::Unavailable::LoadFailed("zip: invalid data".into()))?,
    );

    if entry_count == 0xFFFF || cd_size == 0xFFFFFFFF || cd_offset == 0xFFFFFFFF {
        return Err(crate::runtime::Unavailable::LoadFailed("zip64 is not supported".into()));
    }

    if cd_size == 0 {
        return Err(crate::runtime::Unavailable::LoadFailed("zip: empty central directory".into()));
    }

    let cd_offset_u64 = cd_offset as u64;
    let cd_size_u64 = cd_size as u64;
    let cd_end = cd_offset_u64.checked_add(cd_size_u64).ok_or_else(|| {
        crate::runtime::Unavailable::LoadFailed("zip: central directory out of bounds".into())
    })?;

    if cd_end > total_len {
        return Err(crate::runtime::Unavailable::LoadFailed(
            "zip: central directory out of bounds".into(),
        ));
    }

    let tail_start = total_len.saturating_sub(tail.len() as u64);
    let cd_bytes = if cd_offset_u64 >= tail_start && cd_end <= total_len {
        let start_idx = (cd_offset_u64 - tail_start) as usize;
        let end_idx = start_idx.checked_add(cd_size as usize).ok_or_else(|| {
            crate::runtime::Unavailable::LoadFailed("zip: central directory out of bounds".into())
        })?;
        if start_idx > tail.len() || end_idx > tail.len() {
            return Err(crate::runtime::Unavailable::LoadFailed(
                "zip: central directory out of bounds".into(),
            ));
        }
        tail[start_idx..end_idx].to_vec()
    } else {
        fetch(cd_offset_u64, cd_end - 1)?
    };

    struct CdEntry {
        flags: u16,
        method: u16,
        compressed_size: u32,
        uncompressed_size: u32,
        local_header_offset: u32,
        name: String,
    }

    let mut cd_entries = Vec::new();
    let mut pos = 0;
    while pos + 46 <= cd_bytes.len() {
        if pos + 46 > cd_bytes.len() {
            return Err(crate::runtime::Unavailable::LoadFailed(
                "zip: central directory entry out of bounds".into(),
            ));
        }
        let sig = u32::from_le_bytes(cd_bytes[pos..pos + 4].try_into().map_err(|_| {
            crate::runtime::Unavailable::LoadFailed(
                "zip: invalid central directory signature".into(),
            )
        })?);
        if sig != 0x02014b50 {
            return Err(crate::runtime::Unavailable::LoadFailed(
                "zip: invalid central directory signature".into(),
            ));
        }
        let flags = u16::from_le_bytes(cd_bytes[pos + 8..pos + 10].try_into().map_err(|_| {
            crate::runtime::Unavailable::LoadFailed("zip: invalid central directory flags".into())
        })?);
        let method = u16::from_le_bytes(cd_bytes[pos + 10..pos + 12].try_into().map_err(|_| {
            crate::runtime::Unavailable::LoadFailed("zip: invalid central directory method".into())
        })?);
        let comp_size =
            u32::from_le_bytes(cd_bytes[pos + 20..pos + 24].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid central directory compressed size".into(),
                )
            })?);
        let uncomp_size =
            u32::from_le_bytes(cd_bytes[pos + 24..pos + 28].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid central directory uncompressed size".into(),
                )
            })?);
        let name_len =
            u16::from_le_bytes(cd_bytes[pos + 28..pos + 30].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid central directory name len".into(),
                )
            })?) as usize;
        let extra_len =
            u16::from_le_bytes(cd_bytes[pos + 30..pos + 32].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid central directory extra len".into(),
                )
            })?) as usize;
        let comment_len =
            u16::from_le_bytes(cd_bytes[pos + 32..pos + 34].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid central directory comment len".into(),
                )
            })?) as usize;
        let lho = u32::from_le_bytes(cd_bytes[pos + 42..pos + 46].try_into().map_err(|_| {
            crate::runtime::Unavailable::LoadFailed("zip: invalid central directory lho".into())
        })?);

        if comp_size == 0xFFFFFFFF || uncomp_size == 0xFFFFFFFF || lho == 0xFFFFFFFF {
            return Err(crate::runtime::Unavailable::LoadFailed("zip64 is not supported".into()));
        }

        let name_start = pos + 46;
        let name_end = name_start.checked_add(name_len).ok_or_else(|| {
            crate::runtime::Unavailable::LoadFailed("zip: central directory name overflow".into())
        })?;
        if name_end > cd_bytes.len() {
            return Err(crate::runtime::Unavailable::LoadFailed(
                "zip: central directory entry out of bounds".into(),
            ));
        }
        let name_bytes = &cd_bytes[name_start..name_end];
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        cd_entries.push(CdEntry {
            flags,
            method,
            compressed_size: comp_size,
            uncompressed_size: uncomp_size,
            local_header_offset: lho,
            name,
        });

        let adv1 = 46usize
            .checked_add(name_len)
            .and_then(|v| v.checked_add(extra_len))
            .and_then(|v| v.checked_add(comment_len))
            .ok_or_else(|| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: central directory entry overflow".into(),
                )
            })?;
        pos = pos.checked_add(adv1).ok_or_else(|| {
            crate::runtime::Unavailable::LoadFailed("zip: central directory entry overflow".into())
        })?;
    }

    if cd_entries.len() != entry_count as usize {
        return Err(crate::runtime::Unavailable::LoadFailed(
            "zip: central directory is inconsistent".into(),
        ));
    }

    std::fs::create_dir_all(dest_dir)
        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

    let mut found = vec![false; part.files.len()];

    const MAX_SIZE: u64 = 512 * 1024 * 1024;

    for (idx, pf) in part.files.iter().enumerate() {
        let matched_cd = cd_entries.iter().find(|ce| ce.name.as_bytes() == pf.entry.as_bytes());
        if let Some(ce) = matched_cd {
            if ce.compressed_size as u64 > MAX_SIZE
                || ce.uncompressed_size as u64 > MAX_SIZE
                || ce.local_header_offset as u64 >= total_len
            {
                return Err(crate::runtime::Unavailable::LoadFailed(
                    "zip: unsupported entry".into(),
                ));
            }
            found[idx] = true;
            if (ce.flags & 1) != 0 {
                return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
            }
            let lho = ce.local_header_offset as u64;
            let name_len = ce.name.len() as u64;
            let initial_end_offset = lho
                .checked_add(30)
                .and_then(|v| v.checked_add(name_len))
                .and_then(|v| v.checked_add(1024))
                .and_then(|v| v.checked_add(ce.compressed_size as u64))
                .map(|v| v.saturating_sub(1))
                .unwrap_or(total_len - 1);
            let initial_end = std::cmp::min(initial_end_offset, total_len - 1);
            let mut buf = fetch(lho, initial_end)?;

            if buf.len() < 30 {
                return Err(crate::runtime::Unavailable::LoadFailed(
                    "zip: incomplete local file data".into(),
                ));
            }
            let local_sig = u32::from_le_bytes(buf[0..4].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid local header signature".into(),
                )
            })?);
            if local_sig != 0x04034b50 {
                return Err(crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid local header signature".into(),
                ));
            }
            let l_name_len = u16::from_le_bytes(buf[26..28].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed("zip: invalid local header name len".into())
            })?) as usize;
            let l_extra_len = u16::from_le_bytes(buf[28..30].try_into().map_err(|_| {
                crate::runtime::Unavailable::LoadFailed(
                    "zip: invalid local header extra len".into(),
                )
            })?) as usize;
            let data_start = 30usize
                .checked_add(l_name_len)
                .and_then(|v| v.checked_add(l_extra_len))
                .ok_or_else(|| {
                    crate::runtime::Unavailable::LoadFailed(
                        "zip: invalid local header bounds".into(),
                    )
                })?;
            let required_len =
                data_start.checked_add(ce.compressed_size as usize).ok_or_else(|| {
                    crate::runtime::Unavailable::LoadFailed(
                        "zip: invalid local header bounds".into(),
                    )
                })?;

            if buf.len() < required_len {
                let missing_start = lho.checked_add(buf.len() as u64).ok_or_else(|| {
                    crate::runtime::Unavailable::LoadFailed("zip: local header overflow".into())
                })?;
                let missing_end = lho
                    .checked_add(required_len as u64)
                    .map(|v| v.saturating_sub(1))
                    .unwrap_or(total_len - 1);
                let remainder = fetch(missing_start, std::cmp::min(missing_end, total_len - 1))?;
                buf.extend_from_slice(&remainder);
            }

            if buf.len() < required_len
                || data_start > buf.len()
                || data_start + ce.compressed_size as usize > buf.len()
            {
                return Err(crate::runtime::Unavailable::LoadFailed(
                    "zip: incomplete local file data".into(),
                ));
            }

            let compressed_data = &buf[data_start..data_start + ce.compressed_size as usize];

            let dest_path = dest_dir.join(pf.dest);
            let part_path = dest_dir.join(format!("{}.part", pf.dest));

            if let Some(parent) = part_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
            }

            let mut part_file = std::fs::File::create(&part_path)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

            let mut hasher = Sha256::new();
            let mut size = 0u64;

            let decoder_res: Result<(), crate::runtime::Unavailable> = (|| {
                let mut reader: Box<dyn Read> = match ce.method {
                    0 => Box::new(
                        std::io::Cursor::new(compressed_data).take(ce.uncompressed_size as u64 + 1),
                    ),
                    8 => Box::new(
                        flate2::read::DeflateDecoder::new(std::io::Cursor::new(compressed_data))
                            .take(ce.uncompressed_size as u64 + 1),
                    ),
                    _ => {
                        return Err(crate::runtime::Unavailable::LoadFailed(
                            "zip: unsupported method".into(),
                        ))
                    }
                };

                let mut buf = [0u8; 8192];
                let mut produced = 0u64;
                loop {
                    let n = reader
                        .read(&mut buf)
                        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    produced += n as u64;
                    if produced > ce.uncompressed_size as u64 {
                        return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
                    }
                    size += n as u64;
                    hasher.update(&buf[..n]);
                    part_file
                        .write_all(&buf[..n])
                        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                }
                Ok(())
            })();

            drop(part_file);

            if let Err(e) = decoder_res {
                let _ = std::fs::remove_file(&part_path);
                return Err(e);
            }

            if size != ce.uncompressed_size as u64 || (pf.size != 0 && size != pf.size) {
                let _ = std::fs::remove_file(&part_path);
                return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
            }

            if !pf.sha256.is_empty() {
                let hash = format!("{:x}", hasher.finalize());
                if hash != pf.sha256 {
                    let _ = std::fs::remove_file(&part_path);
                    return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
                }
            }

            std::fs::rename(&part_path, &dest_path)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

            #[cfg(unix)]
            {
                if !pf.sha256.is_empty() {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dest_path, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                }
            }
        } else {
            if !pf.optional {
                return Err(crate::runtime::Unavailable::NotInstalled(pf.dest.into()));
            }
        }
    }

    Ok(())
}

pub fn present_cheap(pkg: &RuntimePackage, dir: &Path) -> bool {
    for f in pinned_files(pkg) {
        let path = dir.join(f.dest);
        if !path.exists() {
            return false;
        }
        if f.size != 0 {
            if let Ok(meta) = path.metadata() {
                if meta.len() != f.size {
                    return false;
                }
            } else {
                return false;
            }
        }
    }
    true
}

fn extract_archive_entry<R: Read>(
    reader: &mut R,
    pf: &PartFile,
    dest_dir: &Path,
) -> Result<(), crate::runtime::Unavailable> {
    let dest_path = dest_dir.join(pf.dest);
    let part_path = dest_dir.join(format!("{}.part", pf.dest));

    if let Some(parent) = part_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
    }

    let mut part_file = std::fs::File::create(&part_path)
        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
        part_file
            .write_all(&buf[..n])
            .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
    }
    drop(part_file);

    if pf.size != 0 && size != pf.size {
        let _ = std::fs::remove_file(&part_path);
        return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
    }

    if !pf.sha256.is_empty() {
        let hash = format!("{:x}", hasher.finalize());
        if hash != pf.sha256 {
            let _ = std::fs::remove_file(&part_path);
            return Err(crate::runtime::Unavailable::Corrupt(pf.dest.into()));
        }
    }

    std::fs::rename(&part_path, &dest_path)
        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

    #[cfg(unix)]
    {
        if !pf.sha256.is_empty() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest_path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
        }
    }

    Ok(())
}

pub fn extract_part(
    part: &Part,
    archive: &Path,
    dest_dir: &Path,
) -> Result<(), crate::runtime::Unavailable> {
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

    let mut found = vec![false; part.files.len()];

    match part.kind {
        ArchiveKind::Tgz => {
            let file = std::fs::File::open(archive)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar_archive = tar::Archive::new(decoder);
            let entries = tar_archive
                .entries()
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

            for entry_result in entries {
                let mut entry = entry_result
                    .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                let path = entry
                    .path()
                    .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                let path_str = path.to_string_lossy().replace('\\', "/");

                if let Some((idx, pf)) =
                    part.files.iter().enumerate().find(|(_, pf)| pf.entry == path_str)
                {
                    found[idx] = true;
                    extract_archive_entry(&mut entry, pf, dest_dir)?;
                }
            }
        }
        ArchiveKind::Zip => {
            let file = std::fs::File::open(archive)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
            let mut zip_archive = zip::ZipArchive::new(file)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;

            for i in 0..zip_archive.len() {
                let mut zip_entry = zip_archive
                    .by_index(i)
                    .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
                let name = zip_entry.name().to_string();

                if let Some((idx, pf)) =
                    part.files.iter().enumerate().find(|(_, pf)| pf.entry == name)
                {
                    found[idx] = true;
                    extract_archive_entry(&mut zip_entry, pf, dest_dir)?;
                }
            }
        }
        ArchiveKind::ZipRange => {
            return Err(crate::runtime::Unavailable::LoadFailed(
                "remote zip needs extract_remote_zip".into(),
            ));
        }
    }

    for (idx, pf) in part.files.iter().enumerate() {
        if !found[idx] && !pf.optional {
            return Err(crate::runtime::Unavailable::NotInstalled(pf.dest.into()));
        }
    }

    Ok(())
}

pub fn verify_installed(
    pkg: &RuntimePackage,
    dir: &Path,
) -> Result<PathBuf, crate::runtime::Unavailable> {
    for f in pinned_files(pkg) {
        let path = dir.join(f.dest);
        if !path.exists() {
            return Err(crate::runtime::Unavailable::NotInstalled(f.dest.into()));
        }
        let mut file = std::fs::File::open(&path)
            .map_err(|_| crate::runtime::Unavailable::NotInstalled(f.dest.into()))?;

        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut buf = [0u8; 8192];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| crate::runtime::Unavailable::LoadFailed(e.to_string()))?;
            if n == 0 {
                break;
            }
            size += n as u64;
            hasher.update(&buf[..n]);
        }

        if f.size != 0 && size != f.size {
            return Err(crate::runtime::Unavailable::Corrupt(f.dest.into()));
        }
        if !f.sha256.is_empty() {
            let hash = format!("{:x}", hasher.finalize());
            if hash != f.sha256 {
                return Err(crate::runtime::Unavailable::Corrupt(f.dest.into()));
            }
        }
    }
    Ok(dir.join(main_lib(pkg).dest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper_build_tgz(
        lib_entry: &str,
        lib_content: &[u8],
        license_entry: &str,
        license_content: &[u8],
    ) -> Vec<u8> {
        let mut tar_data = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_data);

            let mut header = tar::Header::new_gnu();
            header.set_size(lib_content.len() as u64);
            header.set_mode(0o644);
            header.set_path(lib_entry).unwrap();
            header.set_cksum();
            builder.append(&header, lib_content).unwrap();

            let mut header = tar::Header::new_gnu();
            header.set_size(license_content.len() as u64);
            header.set_mode(0o644);
            header.set_path(license_entry).unwrap();
            header.set_cksum();
            builder.append(&header, license_content).unwrap();

            let mut header = tar::Header::new_gnu();
            let evil_content = b"evil";
            let path_bytes = b"../evil.txt";
            let name_field = &mut header.as_mut_bytes()[0..100];
            name_field[..path_bytes.len()].copy_from_slice(path_bytes);
            name_field[path_bytes.len()] = 0;
            header.set_size(evil_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, evil_content.as_slice()).unwrap();

            let mut header = tar::Header::new_gnu();
            let junk_content = b"junk";
            header.set_size(junk_content.len() as u64);
            header.set_mode(0o644);
            header.set_path("junk/readme.txt").unwrap();
            header.set_cksum();
            builder.append(&header, junk_content.as_slice()).unwrap();

            builder.finish().unwrap();
        }

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_tgz_part() {
        let lib_entry = "test-pkg-1.0.0/lib/libtest.so";
        let lib_file = "libtest.so";
        let lib_content = b"hello onnxruntime test library content";
        let mut hasher = Sha256::new();
        hasher.update(lib_content);
        let lib_sha256 = format!("{:x}", hasher.finalize());
        let lib_size = lib_content.len() as u64;

        let license_entry = "test-pkg-1.0.0/LICENSE";
        let license_content = b"MIT License";

        let tgz_bytes = helper_build_tgz(lib_entry, lib_content, license_entry, license_content);

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_tgz_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let archive_path = tmp_dir.join("test.tgz");
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        std::fs::write(&archive_path, &tgz_bytes).unwrap();

        let files = vec![
            PartFile {
                entry: Box::leak(lib_entry.to_string().into_boxed_str()),
                dest: Box::leak(lib_file.to_string().into_boxed_str()),
                size: lib_size,
                sha256: Box::leak(lib_sha256.to_string().into_boxed_str()),
                optional: false,
            },
            PartFile {
                entry: Box::leak(license_entry.to_string().into_boxed_str()),
                dest: "LICENSE",
                size: 0,
                sha256: "",
                optional: true,
            },
        ];
        let files_ref: &'static [PartFile] = Box::leak(files.into_boxed_slice());

        let part = Part {
            name: "test-tgz",
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/test.tgz",
            archive_size: tgz_bytes.len() as u64,
            archive_sha256: "",
            fetch_size: tgz_bytes.len() as u64,
            kind: ArchiveKind::Tgz,
            hosts: GITHUB_HOSTS,
            files: files_ref,
        };

        extract_part(&part, &archive_path, &dest_dir).unwrap();
        let path = dest_dir.join(lib_file);
        assert_eq!(std::fs::read(&path).unwrap(), lib_content);
        assert_eq!(std::fs::read_to_string(dest_dir.join("LICENSE")).unwrap(), "MIT License");

        assert!(!dest_dir.join("evil.txt").exists());
        assert!(!dest_dir.join("readme.txt").exists());
        assert!(!tmp_dir.join("evil.txt").exists());

        let entries = std::fs::read_dir(&dest_dir).unwrap();
        for entry in entries {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().ends_with(".part"));
        }

        let bad_files = vec![PartFile {
            entry: files_ref[0].entry,
            dest: files_ref[0].dest,
            size: files_ref[0].size,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            optional: false,
        }];
        let bad_part = Part { files: Box::leak(bad_files.into_boxed_slice()), ..part.clone() };
        let dest_dir2 = tmp_dir.join("dest2");
        let res = extract_part(&bad_part, &archive_path, &dest_dir2);
        assert_eq!(res.unwrap_err(), crate::runtime::Unavailable::Corrupt(lib_file.into()));
        assert!(!dest_dir2.join(lib_file).exists());

        let tgz_empty = helper_build_tgz("other/path", b"other", license_entry, license_content);
        let archive_empty = tmp_dir.join("empty.tgz");
        std::fs::write(&archive_empty, &tgz_empty).unwrap();
        let dest_dir3 = tmp_dir.join("dest3");
        let res = extract_part(&part, &archive_empty, &dest_dir3);
        assert_eq!(res.unwrap_err(), crate::runtime::Unavailable::NotInstalled(lib_file.into()));

        let part_missing_opt = Part {
            files: Box::leak(vec![files_ref[0].clone()].into_boxed_slice()),
            ..part.clone()
        };
        let dest_dir4 = tmp_dir.join("dest4");
        extract_part(&part_missing_opt, &archive_path, &dest_dir4).unwrap();
        assert!(dest_dir4.join(lib_file).exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_zip_part() {
        let dll_entry = "fake.dll";
        let dll_content = b"fake dll content bytes";
        let mut hasher = Sha256::new();
        hasher.update(dll_content);
        let dll_sha256 = format!("{:x}", hasher.finalize());
        let dll_size = dll_content.len() as u64;

        let license_entry = "licenses/LICENSE.txt";
        let license_content = b"MIT Zip License";

        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            writer.start_file(dll_entry, options).unwrap();
            writer.write_all(dll_content).unwrap();

            writer.start_file(license_entry, options).unwrap();
            writer.write_all(license_content).unwrap();

            writer.start_file("../evil.txt", options).unwrap();
            writer.write_all(b"evil zip").unwrap();

            writer.start_file("unrelated.txt", options).unwrap();
            writer.write_all(b"unrelated").unwrap();

            writer.finish().unwrap();
        }

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_zip_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let archive_path = tmp_dir.join("test.zip");
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        std::fs::write(&archive_path, &zip_bytes).unwrap();

        let files = vec![
            PartFile {
                entry: dll_entry,
                dest: "fake.dll",
                size: dll_size,
                sha256: Box::leak(dll_sha256.to_string().into_boxed_str()),
                optional: false,
            },
            PartFile {
                entry: license_entry,
                dest: "licenses/LICENSE.txt",
                size: 0,
                sha256: "",
                optional: true,
            },
        ];

        let part = Part {
            name: "test-zip",
            url: "https://example.com/test.zip",
            archive_size: zip_bytes.len() as u64,
            archive_sha256: "",
            fetch_size: zip_bytes.len() as u64,
            kind: ArchiveKind::Zip,
            hosts: &[],
            files: Box::leak(files.into_boxed_slice()),
        };

        extract_part(&part, &archive_path, &dest_dir).unwrap();
        assert_eq!(std::fs::read(dest_dir.join("fake.dll")).unwrap(), dll_content);
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("licenses/LICENSE.txt")).unwrap(),
            "MIT Zip License"
        );

        assert!(!dest_dir.join("../evil.txt").exists());
        assert!(!dest_dir.join("unrelated.txt").exists());
        assert!(!tmp_dir.join("evil.txt").exists());

        let bad_files = vec![PartFile {
            entry: dll_entry,
            dest: "fake.dll",
            size: dll_size,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            optional: false,
        }];
        let bad_part = Part { files: Box::leak(bad_files.into_boxed_slice()), ..part.clone() };
        let dest_dir2 = tmp_dir.join("dest2");
        let res = extract_part(&bad_part, &archive_path, &dest_dir2);
        assert_eq!(res.unwrap_err(), crate::runtime::Unavailable::Corrupt("fake.dll".into()));
        assert!(!dest_dir2.join("fake.dll").exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_verify_and_present_cheap() {
        let lib_entry = "lib/libtest.so";
        let lib_content = b"lib content";
        let mut hasher = Sha256::new();
        hasher.update(lib_content);
        let lib_sha256 = format!("{:x}", hasher.finalize());

        let tgz_bytes = helper_build_tgz(lib_entry, lib_content, "LICENSE", b"MIT");

        let zip_entry = "data.bin";
        let zip_content = b"zip content";
        let mut hasher_zip = Sha256::new();
        hasher_zip.update(zip_content);
        let zip_sha256 = format!("{:x}", hasher_zip.finalize());

        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file(zip_entry, options).unwrap();
            writer.write_all(zip_content).unwrap();
            writer.finish().unwrap();
        }

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_multi_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let tgz_path = tmp_dir.join("part1.tgz");
        let zip_path = tmp_dir.join("part2.zip");
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        std::fs::write(&tgz_path, &tgz_bytes).unwrap();
        std::fs::write(&zip_path, &zip_bytes).unwrap();

        let part1 = Part {
            name: "p1",
            url: "https://example.com/1.tgz",
            archive_size: tgz_bytes.len() as u64,
            archive_sha256: "",
            fetch_size: tgz_bytes.len() as u64,
            kind: ArchiveKind::Tgz,
            hosts: &[],
            files: Box::leak(
                vec![PartFile {
                    entry: lib_entry,
                    dest: "libtest.so",
                    size: lib_content.len() as u64,
                    sha256: Box::leak(lib_sha256.to_string().into_boxed_str()),
                    optional: false,
                }]
                .into_boxed_slice(),
            ),
        };

        let part2 = Part {
            name: "p2",
            url: "https://example.com/2.zip",
            archive_size: zip_bytes.len() as u64,
            archive_sha256: "",
            fetch_size: zip_bytes.len() as u64,
            kind: ArchiveKind::Zip,
            hosts: &[],
            files: Box::leak(
                vec![PartFile {
                    entry: zip_entry,
                    dest: "data.bin",
                    size: zip_content.len() as u64,
                    sha256: Box::leak(zip_sha256.to_string().into_boxed_str()),
                    optional: false,
                }]
                .into_boxed_slice(),
            ),
        };

        let pkg = RuntimePackage {
            platform: "multi-test",
            parts: Box::leak(vec![part1.clone(), part2.clone()].into_boxed_slice()),
        };

        assert!(!present_cheap(&pkg, &dest_dir));
        assert!(verify_installed(&pkg, &dest_dir).is_err());

        extract_part(&part1, &tgz_path, &dest_dir).unwrap();
        assert!(!present_cheap(&pkg, &dest_dir));

        extract_part(&part2, &zip_path, &dest_dir).unwrap();
        assert!(present_cheap(&pkg, &dest_dir));
        assert!(verify_installed(&pkg, &dest_dir).is_ok());

        let lib_path = dest_dir.join("libtest.so");
        let mut bytes = std::fs::read(&lib_path).unwrap();
        bytes[0] ^= 0xff;
        std::fs::write(&lib_path, &bytes).unwrap();
        assert!(present_cheap(&pkg, &dest_dir));
        assert_eq!(
            verify_installed(&pkg, &dest_dir).unwrap_err(),
            crate::runtime::Unavailable::Corrupt("libtest.so".into())
        );

        std::fs::write(&lib_path, lib_content).unwrap();
        std::fs::remove_file(dest_dir.join("data.bin")).unwrap();
        assert!(!present_cheap(&pkg, &dest_dir));
        assert_eq!(
            verify_installed(&pkg, &dest_dir).unwrap_err(),
            crate::runtime::Unavailable::NotInstalled("data.bin".into())
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    fn helper_build_remote_zip() -> (Vec<u8>, u64, Vec<u8>, String, Vec<u8>) {
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let dll_content = b"fake dml dll content bytes 18.5 MB simulated";
        let mut hasher = Sha256::new();
        hasher.update(dll_content);
        let dll_sha256 = format!("{:x}", hasher.finalize());

        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let license_content = b"MIT License Windows";

        let mut zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_bytes);
            let mut writer = zip::ZipWriter::new(cursor);
            let options_deflate = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let options_stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            writer.start_file(dll_entry, options_deflate).unwrap();
            writer.write_all(dll_content).unwrap();

            writer.start_file(license_entry, options_stored).unwrap();
            writer.write_all(license_content).unwrap();

            let mut xorshift = 987654321u32;
            let mut junk_data = vec![0u8; 300 * 1024];
            for b in &mut junk_data {
                xorshift ^= xorshift << 13;
                xorshift ^= xorshift >> 17;
                xorshift ^= xorshift << 5;
                *b = xorshift as u8;
            }
            writer.start_file("onnxruntime-win-x64-1.24.4/big_junk.bin", options_deflate).unwrap();
            writer.write_all(&junk_data).unwrap();

            writer.start_file("../evil.txt", options_stored).unwrap();
            writer.write_all(b"evil").unwrap();

            writer.finish().unwrap();
        }

        let total_len = zip_bytes.len() as u64;
        let tail_len = std::cmp::min(total_len, REMOTE_ZIP_TAIL) as usize;
        let tail = zip_bytes[zip_bytes.len() - tail_len..].to_vec();
        (zip_bytes, total_len, tail, dll_sha256, dll_content.to_vec())
    }

    fn helper_remote_zip_part(
        dll_entry: &'static str,
        dll_sha256: &'static str,
        dll_size: u64,
        total_len: u64,
        license_entry: &'static str,
    ) -> (Part, Vec<PartFile>) {
        let files = vec![
            PartFile {
                entry: dll_entry,
                dest: "onnxruntime.dll",
                size: dll_size,
                sha256: dll_sha256,
                optional: false,
            },
            PartFile {
                entry: license_entry,
                dest: "licenses/x.txt",
                size: 0,
                sha256: "",
                optional: true,
            },
            PartFile {
                entry: "nonexistent.txt",
                dest: "missing_opt.txt",
                size: 0,
                sha256: "",
                optional: true,
            },
        ];
        let part = Part {
            name: "test-zip-range",
            url: "https://example.com/test.zip",
            archive_size: total_len,
            archive_sha256: "",
            fetch_size: 1024 * 1024,
            kind: ArchiveKind::ZipRange,
            hosts: &[],
            files: Box::leak(files.clone().into_boxed_slice()),
        };
        (part, files)
    }

    #[test]
    fn test_extract_remote_zip_good() {
        let (zip_bytes, total_len, tail, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_good_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes);
        let fetched_bytes_count = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let zip_rc_clone = zip_rc.clone();
        let fetched_bytes_count_clone = fetched_bytes_count.clone();

        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            if start > end_inclusive {
                return Ok(Vec::new());
            }
            let slice = zip_rc_clone[start..=end_inclusive].to_vec();
            fetched_bytes_count_clone.set(fetched_bytes_count_clone.get() + slice.len());
            Ok(slice)
        };

        extract_remote_zip(&part, total_len, &tail, &dest_dir, &mut fetch_fn).unwrap();
        assert_eq!(std::fs::read(dest_dir.join("onnxruntime.dll")).unwrap(), dll_content);
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("licenses/x.txt")).unwrap(),
            "MIT License Windows"
        );
        assert!(!dest_dir.join("big_junk.bin").exists());
        assert!(!dest_dir.join("evil.txt").exists());
        assert!(fetched_bytes_count.get() < total_len as usize);

        let entries = std::fs::read_dir(&dest_dir).unwrap();
        for entry in entries {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().ends_with(".part"));
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_tiny_tail() {
        let (zip_bytes, total_len, _, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let tiny_tail = zip_bytes[zip_bytes.len() - 30..].to_vec();
        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_tiny_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes);
        let zip_rc_clone = zip_rc.clone();
        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            if start > end_inclusive {
                return Ok(Vec::new());
            }
            Ok(zip_rc_clone[start..=end_inclusive].to_vec())
        };

        extract_remote_zip(&part, total_len, &tiny_tail, &dest_dir, &mut fetch_fn).unwrap();
        assert_eq!(std::fs::read(dest_dir.join("onnxruntime.dll")).unwrap(), dll_content);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_wrong_sha256() {
        let (zip_bytes, total_len, tail, _, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            "0000000000000000000000000000000000000000000000000000000000000000",
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_sha_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes);
        let zip_rc_clone = zip_rc.clone();
        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            Ok(zip_rc_clone[start..=end_inclusive].to_vec())
        };

        let res = extract_remote_zip(&part, total_len, &tail, &dest_dir, &mut fetch_fn);
        assert_eq!(
            res.unwrap_err(),
            crate::runtime::Unavailable::Corrupt("onnxruntime.dll".into())
        );
        assert!(!dest_dir.join("onnxruntime.dll").exists());
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_wrong_total_len() {
        let (_, total_len, tail, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_len_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let fetch_count = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let fetch_count_clone = fetch_count.clone();
        let mut fetch_fn = move |_, _| {
            fetch_count_clone.set(fetch_count_clone.get() + 1);
            Ok(Vec::new())
        };

        let res = extract_remote_zip(&part, total_len + 1, &tail, &dest_dir, &mut fetch_fn);
        assert_eq!(res.unwrap_err(), crate::runtime::Unavailable::Corrupt(part.name.into()));
        assert_eq!(fetch_count.get(), 0);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_missing_entries() {
        let (zip_bytes, total_len, tail, _, _) = helper_build_remote_zip();
        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_missing_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes);
        let zip_rc_clone = zip_rc.clone();
        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            Ok(zip_rc_clone[start..=end_inclusive].to_vec())
        };

        let missing_files = vec![PartFile {
            entry: "nonexistent_required.dll",
            dest: "req.dll",
            size: 0,
            sha256: "",
            optional: false,
        }];
        let missing_part = Part {
            name: "test-zip-range",
            url: "https://example.com/test.zip",
            archive_size: total_len,
            archive_sha256: "",
            fetch_size: 1024,
            kind: ArchiveKind::ZipRange,
            hosts: &[],
            files: Box::leak(missing_files.into_boxed_slice()),
        };
        let res = extract_remote_zip(&missing_part, total_len, &tail, &dest_dir, &mut fetch_fn);
        assert_eq!(res.unwrap_err(), crate::runtime::Unavailable::NotInstalled("req.dll".into()));

        let opt_missing_files = vec![PartFile {
            entry: "nonexistent_optional.txt",
            dest: "opt.txt",
            size: 0,
            sha256: "",
            optional: true,
        }];
        let opt_missing_part =
            Part { files: Box::leak(opt_missing_files.into_boxed_slice()), ..missing_part };
        extract_remote_zip(&opt_missing_part, total_len, &tail, &dest_dir, &mut fetch_fn).unwrap();
        assert!(!dest_dir.join("opt.txt").exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_zip64_marker() {
        let (zip_bytes, total_len, _, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let mut zip_bytes_zip64 = zip_bytes.clone();
        let mut eocd_idx = None;
        for i in (0..=zip_bytes_zip64.len() - 22).rev() {
            if u32::from_le_bytes(zip_bytes_zip64[i..i + 4].try_into().unwrap()) == 0x06054b50 {
                eocd_idx = Some(i);
                break;
            }
        }
        if let Some(idx) = eocd_idx {
            zip_bytes_zip64[idx + 16..idx + 20].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        }
        let tail64_len = std::cmp::min(total_len, REMOTE_ZIP_TAIL) as usize;
        let tail64 = zip_bytes_zip64[zip_bytes_zip64.len() - tail64_len..].to_vec();

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_zip64_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes_zip64);
        let zip_rc_clone = zip_rc.clone();
        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            Ok(zip_rc_clone[start..=end_inclusive].to_vec())
        };

        let res = extract_remote_zip(&part, total_len, &tail64, &dest_dir, &mut fetch_fn);
        assert!(matches!(res, Err(crate::runtime::Unavailable::LoadFailed(_))), "{res:?}");
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_cd_size_zero() {
        let (zip_bytes, total_len, _, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let mut zip_bytes_zero = zip_bytes.clone();
        let mut eocd_idx = None;
        for i in (0..=zip_bytes_zero.len() - 22).rev() {
            if u32::from_le_bytes(zip_bytes_zero[i..i + 4].try_into().unwrap()) == 0x06054b50 {
                eocd_idx = Some(i);
                break;
            }
        }
        if let Some(idx) = eocd_idx {
            zip_bytes_zero[idx + 12..idx + 16].copy_from_slice(&0u32.to_le_bytes());
        }
        let tail_len = std::cmp::min(total_len, REMOTE_ZIP_TAIL) as usize;
        let tail_zero = zip_bytes_zero[zip_bytes_zero.len() - tail_len..].to_vec();

        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_cdzero_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let zip_rc = std::rc::Rc::new(zip_bytes_zero);
        let zip_rc_clone = zip_rc.clone();
        let mut fetch_fn = move |start: u64, end_inclusive: u64| {
            let start = start as usize;
            let end_inclusive = (end_inclusive as usize).min(zip_rc_clone.len() - 1);
            Ok(zip_rc_clone[start..=end_inclusive].to_vec())
        };

        let res = extract_remote_zip(&part, total_len, &tail_zero, &dest_dir, &mut fetch_fn);
        assert!(matches!(res, Err(crate::runtime::Unavailable::LoadFailed(_))), "{res:?}");
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_remote_zip_truncated_tail() {
        let (_, total_len, _, dll_sha256, dll_content) = helper_build_remote_zip();
        let dll_entry = "onnxruntime-win-x64-1.24.4/runtimes/win-x64/native/onnxruntime.dll";
        let license_entry = "onnxruntime-win-x64-1.24.4/LICENSE";
        let (part, _) = helper_remote_zip_part(
            dll_entry,
            Box::leak(dll_sha256.into_boxed_str()),
            dll_content.len() as u64,
            total_len,
            license_entry,
        );

        let garbled_tail = b"shorttail".to_vec();
        let tmp_dir = std::env::temp_dir().join(format!(
            "karaoke_test_remotezip_garbled_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let dest_dir = tmp_dir.join("dest");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let mut fetch_fn = move |_, _| Ok(Vec::new());

        let res = extract_remote_zip(&part, total_len, &garbled_tail, &dest_dir, &mut fetch_fn);
        assert_eq!(
            res.unwrap_err(),
            crate::runtime::Unavailable::LoadFailed("zip: no end record".into())
        );
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_pinned_packages_sanity() {
        for pkg in &[&LINUX_X86_64, &MACOS_AARCH64, &WINDOWS_X86_64] {
            assert!(total_archive_size(pkg) > 0);
            let main = main_lib(pkg);
            if pkg.platform.starts_with("windows") {
                assert_eq!(main.dest, "onnxruntime.dll");
            } else {
                assert!(main.dest.contains(VERSION));
            }

            if pkg.platform == "windows-x86_64" {
                assert_eq!(total_archive_size(pkg), 12458649 + 9420000);
                let pinned: Vec<_> = pinned_files(pkg).collect();
                assert_eq!(pinned.len(), 2);
                assert_eq!(pinned[0].dest, "onnxruntime.dll");
                assert_eq!(pinned[1].dest, "DirectML.dll");
            }

            for (i, part) in pkg.parts.iter().enumerate() {
                if pkg.platform.starts_with("windows") {
                    assert_eq!(part.hosts, NUGET_HOSTS);
                    assert!(part.url.starts_with("https://api.nuget.org/v3-flatcontainer/"));
                    assert!(part.url.ends_with(".nupkg"));

                    if i == 0 {
                        assert_eq!(part.kind, ArchiveKind::Zip);
                        assert_eq!(part.archive_sha256.len(), 64);
                        assert!(part
                            .archive_sha256
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
                        assert_eq!(part.fetch_size, part.archive_size);
                    } else if i == 1 {
                        assert_eq!(part.kind, ArchiveKind::ZipRange);
                        assert!(part.archive_sha256.is_empty());
                        assert!(part.fetch_size < part.archive_size);
                    }
                } else {
                    assert_eq!(part.archive_sha256.len(), 64);
                    assert!(part
                        .archive_sha256
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
                    assert!(part.hosts.contains(&"github.com"));

                    assert!(part.fetch_size > 0);
                    assert_eq!(part.fetch_size, part.archive_size);

                    let prefix = format!(
                        "https://github.com/microsoft/onnxruntime/releases/download/v{VERSION}/"
                    );
                    assert!(part.url.starts_with(&prefix));
                    assert!(part.url.ends_with(".tgz"));
                }

                assert!(part.files.iter().any(|f| !f.sha256.is_empty() && !f.optional));

                for f in part.files {
                    if !f.sha256.is_empty() && !f.optional {
                        assert_eq!(f.sha256.len(), 64);
                        assert!(f
                            .sha256
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
                        assert!(f.size > 0);
                    }
                }
            }
        }
    }
}
