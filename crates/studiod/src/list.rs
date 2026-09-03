//! 列出作品。
//!
//! 没有中央注册表——**目录就是事实源**。这里做的事就是扫一遍目录，
//! 把每个 bundle 的 `project.toml` 和状态库读出来。所以「删除一部作品」
//! 就是 `rm -rf`，「另存一版」就是 `cp -r`，不需要程序参与。
//!
//! 作品的标识是它的路径。没有 id，因为不需要 id。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use studio_core::{StageId, StageState};
use studio_engine::bundle::DB_FILE;
use studio_store::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub title: String,
    pub path: String,
    pub stage: String,
    pub status: String,
    pub completed: usize,
    pub total: usize,
    pub updated_at: Option<String>,
    /// 打不开时说明原因，而不是把这一行悄悄吞掉。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// 在 `roots` 下面找作品。深度有限——作品目录通常是平铺的，
/// 没必要把整个家目录翻一遍。
pub fn scan(roots: &[PathBuf], depth: usize) -> Vec<Entry> {
    let mut found = Vec::new();
    for root in roots {
        walk(root, depth, &mut found);
    }
    found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.path.cmp(&b.path)));
    found
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    if dir.join(DB_FILE).is_file() {
        out.push(read(dir));
        return; // 作品不嵌套
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        // 不下钻到隐藏目录和明显的非作品目录
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        walk(&p, depth - 1, out);
    }
}

fn read(dir: &Path) -> Entry {
    let path = dir.display().to_string();
    let store = match Store::open(&dir.join(DB_FILE)) {
        Ok(s) => s,
        Err(e) => {
            return Entry {
                title: dir
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path,
                stage: "-".into(),
                status: "打不开".into(),
                completed: 0,
                total: 9,
                updated_at: None,
                problem: Some(format!("{} —— {}", e.message(), e.remedy())),
            };
        }
    };

    let title = store.title().unwrap_or_else(|_| "未命名作品".into());
    let mut completed = 0;
    let mut current = StageId::Review;
    let mut seen_current = false;
    for s in StageId::all() {
        match store.load_stage(s).map(|l| l.state()) {
            Ok(StageState::Approved) => completed += 1,
            Ok(_) if !seen_current => {
                current = s;
                seen_current = true;
            }
            _ => {}
        }
    }

    let pending = store.pending_question().ok().flatten();
    let status = if !seen_current {
        "已完成".to_string()
    } else if pending.is_some() {
        "等确认".to_string()
    } else if store
        .meta("stage_error")
        .ok()
        .flatten()
        .is_some_and(|v| !v.is_empty())
    {
        "阻塞".to_string()
    } else {
        "进行中".to_string()
    };

    let updated_at = store
        .timeline(1)
        .ok()
        .and_then(|t| t.first().map(|e| e.at.clone()));

    Entry {
        title,
        path,
        stage: if seen_current {
            current.as_str().to_string()
        } else {
            "-".into()
        },
        status,
        completed,
        total: 9,
        updated_at,
        problem: None,
    }
}

pub fn render(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "没找到作品。\n  新建一部：studiod init ~/videos/我的第一部.studio\n".to_string();
    }
    let w_title = entries
        .iter()
        .map(|e| width(&e.title))
        .max()
        .unwrap_or(4)
        .clamp(4, 32);
    let w_status = entries
        .iter()
        .map(|e| width(&e.status))
        .max()
        .unwrap_or(6)
        .max(width("状态"));
    let mut s = String::new();
    s.push_str(&pad("名称", w_title));
    s.push_str("  ");
    s.push_str(&pad("状态", w_status));
    s.push_str(&format!(
        "  {:<14}  {:<7}  {:<20}  {}\n",
        "阶段", "进度", "最近活动", "目录"
    ));
    s.push_str(&"-".repeat(w_title + w_status + 52));
    s.push('\n');
    for e in entries {
        s.push_str(&pad(&truncate(&e.title, w_title), w_title));
        s.push_str("  ");
        s.push_str(&pad(&e.status, w_status));
        s.push_str(&format!(
            "  {:<14}  {:>3}/{:<3}  {:<20}  {}\n",
            e.stage,
            e.completed,
            e.total,
            e.updated_at.as_deref().unwrap_or("-"),
            e.path
        ));
        if let Some(p) = &e.problem {
            s.push_str(&format!("{}  {p}\n", " ".repeat(w_title)));
        }
    }
    s
}

/// 按显示宽度右侧补空格。中文占两列，用字符数补会歪。
fn pad(s: &str, to: usize) -> String {
    let mut out = s.to_string();
    out.push_str(&" ".repeat(to.saturating_sub(width(s))));
    out
}

/// 中文按两列宽算，表格才对得齐。
fn width(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) > 0x2E80 { 2 } else { 1 })
        .sum()
}

fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = if (c as u32) > 0x2E80 { 2 } else { 1 };
        if w + cw > max.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_bundles_and_skips_plain_directories() {
        let d = tempfile::tempdir().unwrap();
        studio_engine::init_project(&d.path().join("甲.studio"), "甲作品", "0.1.0", &[]).unwrap();
        studio_engine::init_project(&d.path().join("乙.studio"), "乙作品", "0.1.0", &[]).unwrap();
        std::fs::create_dir_all(d.path().join("随便一个目录")).unwrap();

        let entries = scan(&[d.path().to_path_buf()], 2);
        assert_eq!(entries.len(), 2);
        let titles: Vec<&str> = entries.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"甲作品") && titles.contains(&"乙作品"));
        assert!(entries
            .iter()
            .all(|e| e.stage == "idea" && e.completed == 0 && e.problem.is_none()));
    }

    #[test]
    fn a_broken_bundle_is_listed_with_the_reason() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join("坏的.studio");
        std::fs::create_dir_all(b.join(".studio")).unwrap();
        std::fs::write(b.join(DB_FILE), "这不是 sqlite").unwrap();
        let entries = scan(&[d.path().to_path_buf()], 2);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "打不开");
        assert!(
            entries[0].problem.is_some(),
            "打不开的作品要说明原因，不能悄悄吞掉"
        );
    }

    #[test]
    fn nested_bundles_are_not_descended_into() {
        let d = tempfile::tempdir().unwrap();
        let outer = d.path().join("外.studio");
        studio_engine::init_project(&outer, "外", "0.1.0", &[]).unwrap();
        studio_engine::init_project(&outer.join("媒体里的.studio"), "内", "0.1.0", &[]).unwrap();
        let entries = scan(&[d.path().to_path_buf()], 3);
        assert_eq!(entries.len(), 1, "作品不嵌套，找到一个就不再往里走");
    }

    #[test]
    fn empty_scan_tells_you_how_to_make_one() {
        assert!(render(&[]).contains("studiod init"));
    }

    #[test]
    fn cjk_titles_line_up() {
        assert_eq!(width("千岛湖"), 6);
        assert_eq!(width("abc"), 3);
        assert_eq!(truncate("千岛湖，把快乐装进十秒", 8), "千岛湖…");
    }
}
