//! bundle 布局与进程锁。
//!
//! 一个文件夹就是一部作品。**内部一律相对路径**——这是 `mv` / `cp -r` 之后
//! 还能用的唯一前提，也是「绿色软件」的实质含义。

use fs2::FileExt;
use std::fs;
use std::path::{Path, PathBuf};
use studio_core::{Result, StudioError};

/// bundle 里的固定路径。全部相对 `root`。
pub const STUDIO_DIR: &str = ".studio";
pub const DB_FILE: &str = ".studio/studio.db";
pub const LOCK_FILE: &str = ".studio/studiod.lock";
pub const LOG_FILE: &str = ".studio/logs/studiod.log";
pub const PROJECT_TOML: &str = "project.toml";
pub const AGENTS_MD: &str = "AGENTS.md";
pub const SKILLS_DIR: &str = ".agents/skills";
pub const CODEX_CONFIG: &str = ".codex/config.toml";
pub const STAGES_DIR: &str = "stages";
pub const MEDIA_DIR: &str = "media";
pub const OUTPUT_DIR: &str = "output";

#[derive(Debug, Clone)]
pub struct Bundle {
    root: PathBuf,
}

impl Bundle {
    /// 打开一个已有 bundle。不是作品就报 `not_a_project`。
    pub fn open(root: impl AsRef<Path>) -> Result<Bundle> {
        let root = root.as_ref().to_path_buf();
        if !root.join(DB_FILE).is_file() {
            return Err(StudioError::NotAProject {
                path: root.display().to_string(),
            });
        }
        Ok(Bundle { root })
    }

    /// 从当前目录向上找最近的 bundle。Codex 在作品目录里启动 studiod，
    /// 但用户可能在子目录里，所以往上找一层是合理的便利。
    pub fn discover(start: impl AsRef<Path>) -> Result<Bundle> {
        let mut dir = start.as_ref().to_path_buf();
        loop {
            if dir.join(DB_FILE).is_file() {
                return Ok(Bundle { root: dir });
            }
            if !dir.pop() {
                return Err(StudioError::NotAProject {
                    path: start.as_ref().display().to_string(),
                });
            }
        }
    }

    /// 建立目录骨架。数据库由调用方随后创建。
    pub fn scaffold(root: impl AsRef<Path>) -> Result<Bundle> {
        let root = root.as_ref().to_path_buf();
        for d in [
            STUDIO_DIR,
            ".studio/logs",
            ".agents/skills",
            ".codex",
            STAGES_DIR,
            MEDIA_DIR,
            OUTPUT_DIR,
        ] {
            fs::create_dir_all(root.join(d))
                .map_err(|e| StudioError::internal(format!("建目录 {d} 失败：{e}")))?;
        }
        Ok(Bundle { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_path(&self) -> PathBuf {
        self.root.join(DB_FILE)
    }
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }
    pub fn log_path(&self) -> PathBuf {
        self.root.join(LOG_FILE)
    }
    pub fn stages_dir(&self) -> PathBuf {
        self.root.join(STAGES_DIR)
    }
    pub fn media_dir(&self) -> PathBuf {
        self.root.join(MEDIA_DIR)
    }
    pub fn output_dir(&self) -> PathBuf {
        self.root.join(OUTPUT_DIR)
    }
    pub fn project_toml(&self) -> PathBuf {
        self.root.join(PROJECT_TOML)
    }

    /// 把 bundle 内的相对路径解析成绝对路径。拒绝越界。
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.starts_with('/') || rel.contains("..") {
            return Err(StudioError::internal(format!(
                "bundle 内路径必须是相对且不越界的：{rel}"
            )));
        }
        Ok(self.root.join(rel))
    }

    /// 写一个 bundle 内文件，自动建父目录。
    pub fn write(&self, rel: &str, content: &str) -> Result<()> {
        let p = self.resolve(rel)?;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| StudioError::internal(format!("建目录失败：{e}")))?;
        }
        fs::write(&p, content).map_err(|e| StudioError::internal(format!("写 {rel} 失败：{e}")))
    }

    pub fn read(&self, rel: &str) -> Result<String> {
        let p = self.resolve(rel)?;
        fs::read_to_string(&p).map_err(|_| StudioError::ArtifactMissing {
            path: rel.to_string(),
        })
    }

    /// 独占本 bundle。锁随进程退出自动释放，不存在残留。
    pub fn lock(&self) -> Result<LockGuard> {
        let path = self.lock_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| StudioError::internal(format!("建锁目录失败：{e}")))?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| StudioError::internal(format!("打开锁文件失败：{e}")))?;

        if file.try_lock_exclusive().is_err() {
            let holder = fs::read_to_string(&path).unwrap_or_default();
            let (pid, since) = parse_holder(&holder);
            return Err(StudioError::ProjectBusy { pid, since });
        }

        let stamp = format!(
            "pid={}\nsince={}\n",
            std::process::id(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        // 锁已经拿到，写入持有者信息只为让下一个来的人看到是谁占着。
        let _ = fs::write(&path, stamp);
        Ok(LockGuard { file })
    }
}

fn parse_holder(s: &str) -> (Option<u32>, Option<String>) {
    let mut pid = None;
    let mut since = None;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("pid=") {
            pid = v.trim().parse().ok();
        }
        if let Some(v) = line.strip_prefix("since=") {
            since = Some(v.trim().to_string());
        }
    }
    (pid, since)
}

/// 持有期间独占 bundle；drop 即释放。
#[derive(Debug)]
pub struct LockGuard {
    file: fs::File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_rejects_a_plain_directory() {
        let d = tempfile::tempdir().unwrap();
        let e = Bundle::open(d.path()).unwrap_err();
        assert_eq!(e.code(), "not_a_project");
        assert!(e.remedy().contains("提醒用户"));
        assert!(!e.remedy().contains("studiod"));
    }

    #[test]
    fn scaffold_creates_the_expected_layout() {
        let d = tempfile::tempdir().unwrap();
        let b = Bundle::scaffold(d.path()).unwrap();
        for rel in [
            STUDIO_DIR,
            ".studio/logs",
            SKILLS_DIR,
            ".codex",
            STAGES_DIR,
            MEDIA_DIR,
            OUTPUT_DIR,
        ] {
            assert!(b.root().join(rel).is_dir(), "缺少目录 {rel}");
        }
    }

    #[test]
    fn resolve_refuses_absolute_and_escaping_paths() {
        let d = tempfile::tempdir().unwrap();
        let b = Bundle::scaffold(d.path()).unwrap();
        assert!(b.resolve("/etc/passwd").is_err());
        assert!(b.resolve("../outside").is_err());
        assert!(b.resolve("stages/script.json").is_ok());
    }

    #[test]
    fn second_lock_reports_who_holds_it() {
        let d = tempfile::tempdir().unwrap();
        let b = Bundle::scaffold(d.path()).unwrap();
        let _g = b.lock().unwrap();
        let e = b.lock().unwrap_err();
        assert_eq!(e.code(), "project_busy");
        match e {
            StudioError::ProjectBusy { pid, .. } => assert_eq!(pid, Some(std::process::id())),
            other => panic!("实际 {other}"),
        }
    }

    #[test]
    fn lock_is_released_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let b = Bundle::scaffold(d.path()).unwrap();
        {
            let _g = b.lock().unwrap();
        }
        // 前一个 guard 已经 drop，这里必须能重新拿到——锁不会残留。
        let _g2 = b.lock().expect("锁应当随进程/guard 释放，不留残骸");
    }

    #[test]
    fn discover_walks_up_from_a_subdirectory() {
        let d = tempfile::tempdir().unwrap();
        let b = Bundle::scaffold(d.path()).unwrap();
        std::fs::write(b.db_path(), "").unwrap();
        let deep = b.root().join("media");
        let found = Bundle::discover(&deep).unwrap();
        assert_eq!(found.root(), b.root());
    }
}
