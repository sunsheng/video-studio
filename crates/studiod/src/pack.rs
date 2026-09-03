//! 打包与解包。
//!
//! 工作格式是 bundle 目录，交换格式是单个 `.dvs` 文件。
//! `--no-media` 只带走 stages 与状态，用来做一份轻量的分叉起点。

use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;

pub struct PackStats {
    pub files: usize,
    pub bytes: u64,
    pub skipped_media: usize,
}

/// 打包一部作品。锁文件与 trace 不进包。
pub fn pack(bundle: &Path, out: &Path, include_media: bool) -> std::io::Result<PackStats> {
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut stats = PackStats { files: 0, bytes: 0, skipped_media: 0 };

    for entry in walkdir::WalkDir::new(bundle).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = match path.strip_prefix(bundle) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if rel_str.starts_with(".studio/studiod.lock") || rel_str.starts_with(".studio/trace.jsonl") {
            continue;
        }
        if !include_media && (rel_str.starts_with("media/") || rel_str.starts_with("output/")) {
            stats.skipped_media += 1;
            continue;
        }

        zip.start_file(&rel_str, opts)?;
        let mut f = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        zip.write_all(&buf)?;
        stats.files += 1;
        stats.bytes += buf.len() as u64;
    }
    zip.finish()?;
    Ok(stats)
}

/// 解包成一个 bundle 目录。目标已存在则拒绝，不覆盖。
pub fn unpack(archive: &Path, into: &Path) -> std::io::Result<usize> {
    if into.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} 已存在。换个目标路径，或者先把它移走。", into.display()),
        ));
    }
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut n = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let dest = into.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_bundle() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (rel, body) in [
            ("AGENTS.md", "契约"),
            ("stages/script.json", "{}"),
            (".studio/studio.db", "db"),
            (".studio/studiod.lock", "pid=1"),
            (".studio/trace.jsonl", "{}"),
            ("media/sh01.mp4", "很大的视频"),
            ("output/final.mp4", "成片"),
        ] {
            let p = d.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        d
    }

    #[test]
    fn pack_excludes_the_lock_and_trace() {
        let b = fake_bundle();
        let out = b.path().parent().unwrap().join("a.dvs");
        let stats = pack(b.path(), &out, true).unwrap();
        assert_eq!(stats.files, 5, "锁文件与 trace 不该进包");

        let f = std::fs::File::open(&out).unwrap();
        let zip = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = zip.file_names().map(String::from).collect();
        assert!(names.contains(&"AGENTS.md".to_string()));
        assert!(names.contains(&".studio/studio.db".to_string()));
        assert!(!names.iter().any(|n| n.contains("studiod.lock")));
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn no_media_leaves_the_heavy_files_behind() {
        let b = fake_bundle();
        let out = b.path().parent().unwrap().join("b.dvs");
        let stats = pack(b.path(), &out, false).unwrap();
        assert_eq!(stats.skipped_media, 2);
        assert_eq!(stats.files, 3);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn unpack_restores_the_layout_and_refuses_to_overwrite() {
        let b = fake_bundle();
        let out = b.path().parent().unwrap().join("c.dvs");
        pack(b.path(), &out, true).unwrap();

        let target = b.path().parent().unwrap().join("restored.studio");
        let n = unpack(&out, &target).unwrap();
        assert_eq!(n, 5);
        assert_eq!(std::fs::read_to_string(target.join("AGENTS.md")).unwrap(), "契约");
        assert!(target.join(".studio/studio.db").is_file());

        let e = unpack(&out, &target).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);

        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_dir_all(target);
    }
}
