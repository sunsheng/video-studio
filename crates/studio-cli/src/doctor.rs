//! 体检。
//!
//! 运行本程序的机器**不需要 GPU**，但需要 ffmpeg / ffprobe，
//! 并且需要至少一个可达的 ComfyUI 节点（可以在另一台主机上）。
//! 缺什么、去哪配、配好怎么验证，都在这里说清楚。

use serde::{Deserialize, Serialize};
use studio_comfy::{Comfy, NodeHealth};
use studio_engine::Settings;
use studio_media::{Media, ToolStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub level: Level,
    pub detail: String,
    /// 不通过时怎么修。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub healthy: bool,
    pub program_dir: Option<String>,
    pub bundle: Option<String>,
    pub checks: Vec<Check>,
    pub tools: Vec<ToolStatus>,
    pub nodes: Vec<NodeHealth>,
}

pub fn run(program_dir: Option<&std::path::Path>, bundle: Option<&std::path::Path>) -> Report {
    let settings = Settings::load(program_dir, bundle);
    let media = Media::new(&settings);
    let mut checks = Vec::new();
    let mut tools = Vec::new();

    for tool in ["ffmpeg", "ffprobe"] {
        let st = media.probe_tool(tool);
        if st.found {
            checks.push(Check {
                name: format!("{tool} 可用"),
                level: Level::Ok,
                detail: format!(
                    "{}{}",
                    st.path.clone().unwrap_or_default(),
                    st.version
                        .as_ref()
                        .map(|v| format!("（{v}）"))
                        .unwrap_or_default()
                ),
                remedy: None,
            });
        } else {
            checks.push(Check {
                // 前六个阶段（创意到提示词）不碰媒体，所以这不是硬阻塞。
                name: format!("{tool} 缺失"),
                level: Level::Warn,
                detail: format!("找过：{}", st.looked_in.join("、")),
                remedy: Some(format!(
                    "后期阶段才需要它，创作阶段不受影响。装好 {tool} 后，\n  \
                     或者把它的完整路径写进 .env：\n    {}_PATH=/你的路径/{tool}\n  \
                     它不要求在 PATH 中。bundle 里的 .env 优先于程序目录的 .env。",
                    tool.to_uppercase()
                )),
            });
        }
        tools.push(st);
    }

    let comfy = Comfy::from_settings(&settings);
    let nodes = comfy.health();
    let reachable = nodes.iter().filter(|n| n.reachable).count();
    if reachable > 0 {
        checks.push(Check {
            name: "ComfyUI 节点".into(),
            level: Level::Ok,
            detail: format!("{reachable}/{} 个可达", nodes.len()),
            remedy: None,
        });
    } else {
        checks.push(Check {
            name: "ComfyUI 节点全部不可达".into(),
            level: Level::Warn,
            detail: format!("试过：{}", comfy.nodes().join("、")),
            remedy: Some(
                "渲染之前必须至少有一个可达节点。在 .env 里配：\n    \
                 COMFY_NODES=http://主机:9001,http://主机:9002\n  \
                 本机不需要 GPU，节点可以在另一台机器上。\n  \
                 提交给 ComfyUI 之前的六个阶段不受影响，现在就可以开始创作。"
                    .into(),
            ),
        });
    }

    checks.push(check_image_baselines(program_dir));

    if let Some(root) = bundle {
        checks.push(check_codex_config(root));
    }

    let healthy = !checks.iter().any(|c| c.level == Level::Fail);
    Report {
        healthy,
        program_dir: program_dir.map(|p| p.display().to_string()),
        bundle: bundle.map(|p| p.display().to_string()),
        checks,
        tools,
        nodes,
    }
}

/// 卡片能不能生成，取决于 `z_image` 的两条基线在不在、核验过没有。
///
/// 这一项**永远不是 Fail**：视觉资产阶段可以只提交计划，前六个阶段更是
/// 完全不受影响。它存在的意义是把「计划能提交」和「图能生出来」分开说清楚，
/// 免得有人看着一份通过校验的资产计划，以为卡片已经有了。
fn check_image_baselines(program_dir: Option<&std::path::Path>) -> Check {
    let dir = program_dir
        .map(|p| p.join("assets/workflows/z_image"))
        .unwrap_or_else(|| std::path::PathBuf::from("assets/workflows/z_image"));

    let mut missing = Vec::new();
    let mut unverified = Vec::new();
    for name in ["t2i", "edit"] {
        let path = dir.join(format!("{name}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            missing.push(name);
            continue;
        };
        let verified = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("_studio")?.get("bindings_verified")?.as_bool())
            .unwrap_or(false);
        if !verified {
            unverified.push(name);
        }
    }

    if missing.is_empty() && unverified.is_empty() {
        return Check {
            name: "卡片生成基线".into(),
            level: Level::Ok,
            detail: "z_image 的 t2i 与 edit 都在，且已核验".into(),
            remedy: None,
        };
    }

    let mut detail = Vec::new();
    if !missing.is_empty() {
        detail.push(format!("缺 {}", missing.join(" 与 ")));
    }
    if !unverified.is_empty() {
        detail.push(format!("{} 未核验", unverified.join(" 与 ")));
    }
    Check {
        name: "卡片生成基线未就绪".into(),
        level: Level::Warn,
        detail: detail.join("；"),
        remedy: Some(format!(
            "角色卡/场景卡现在只能提交**计划**，生不出图——计划里的 status 会一直是 planned。\n  \
             其余阶段完全不受影响。要真出图，在装了 ComfyUI 的机器上按\n    {}/README.md\n  \
             导出两条基线，核验过再把 bindings_verified 改成 true。",
            dir.display()
        )),
    }
}

/// 换机器或换安装位置之后，`.codex/config.toml` 里的路径会失效。
fn check_codex_config(root: &std::path::Path) -> Check {
    let path = root.join(".codex/config.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Check {
            name: "作品的 Codex 配置缺失".into(),
            level: Level::Fail,
            detail: format!("{} 不存在", path.display()),
            remedy: Some("跑 `studio-cli doctor --fix` 重新生成。".into()),
        };
    };
    // 路径可能是双引号串也可能是单引号字面量串（Windows 路径用后者）。
    let referenced = toml::from_str::<toml::Value>(&text).ok().and_then(|v| {
        v.get("mcp_servers")?
            .get("video-studio")?
            .get("command")?
            .as_str()
            .map(String::from)
    });
    match referenced {
        Some(p) if std::path::Path::new(&p).is_file() => Check {
            name: "作品的 Codex 配置".into(),
            level: Level::Ok,
            detail: format!("指向 {p}"),
            remedy: None,
        },
        Some(p) => Check {
            name: "作品指向的程序不存在".into(),
            level: Level::Fail,
            detail: format!("配置里写的是 {p}"),
            remedy: Some(
                "这部作品是在别的机器或别的安装位置建的。跑 `studio-cli doctor --fix` \
                 把它改成当前程序的路径——bundle 的其它内容都是相对路径，可以照常用。"
                    .into(),
            ),
        },
        None => Check {
            name: "作品的 Codex 配置不完整".into(),
            level: Level::Fail,
            detail: "找不到 command 行".into(),
            remedy: Some("跑 `studio-cli doctor --fix` 重新生成。".into()),
        },
    }
}

/// 把 `.codex/config.toml` 里的程序路径改成当前二进制。
pub fn fix_codex_config(root: &std::path::Path, studiod_path: &str) -> std::io::Result<()> {
    let dir = root.join(".codex");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("config.toml"),
        crate::assets::codex_config(studiod_path),
    )
}

pub fn render(report: &Report) -> String {
    let mut s = String::new();
    s.push_str("video-studio 体检\n\n");
    if let Some(p) = &report.program_dir {
        s.push_str(&format!("  程序目录  {p}\n"));
    }
    match &report.bundle {
        Some(b) => s.push_str(&format!("  当前作品  {b}\n")),
        None => s.push_str("  当前作品  （不在作品目录里，只检查全局环境）\n"),
    }
    s.push('\n');
    for c in &report.checks {
        let mark = match c.level {
            Level::Ok => "OK  ",
            Level::Warn => "注意",
            Level::Fail => "缺失",
        };
        s.push_str(&format!("  [{mark}] {}\n         {}\n", c.name, c.detail));
        if let Some(r) = &c.remedy {
            for line in r.lines() {
                s.push_str(&format!("         {line}\n"));
            }
        }
        s.push('\n');
    }
    let warnings = report
        .checks
        .iter()
        .filter(|c| c.level == Level::Warn)
        .count();
    if !report.healthy {
        s.push_str("结论：有必须先解决的问题，见上面的「缺失」项。\n");
    } else if warnings > 0 {
        s.push_str("结论：现在就可以开始创作。\n");
        s.push_str("      上面 ");
        s.push_str(&warnings.to_string());
        s.push_str(" 项「注意」只影响渲染和后期，可以边做边补。\n");
    } else {
        s.push_str("结论：全流程就绪。\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn baseline(dir: &std::path::Path, name: &str, verified: bool) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = std::fs::File::create(dir.join(format!("{name}.json"))).unwrap();
        write!(
            f,
            r#"{{ "_studio": {{ "bindings": {{}}, "bindings_verified": {verified} }} }}"#
        )
        .unwrap();
    }

    #[test]
    fn a_missing_image_baseline_is_a_warning_not_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let c = check_image_baselines(Some(tmp.path()));
        assert_eq!(
            c.level,
            Level::Warn,
            "卡片生不出来不该挡住前六个阶段：{c:?}"
        );
        assert!(c.detail.contains("t2i"));
        // 说清「计划能提交」和「图能生出来」是两回事。
        assert!(c.remedy.unwrap().contains("planned"));
    }

    #[test]
    fn an_unverified_baseline_still_counts_as_not_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("assets/workflows/z_image");
        baseline(&dir, "t2i", true);
        baseline(&dir, "edit", false);
        let c = check_image_baselines(Some(tmp.path()));
        assert_eq!(c.level, Level::Warn);
        assert!(c.detail.contains("edit 未核验"), "{}", c.detail);
    }

    #[test]
    fn both_verified_baselines_report_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("assets/workflows/z_image");
        baseline(&dir, "t2i", true);
        baseline(&dir, "edit", true);
        assert_eq!(check_image_baselines(Some(tmp.path())).level, Level::Ok);
    }

    fn write_codex_config(root: &std::path::Path, toml_text: &str) {
        let dir = root.join(".codex");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), toml_text).unwrap();
    }

    #[test]
    fn missing_codex_config_is_a_fail_with_a_fix_command() {
        let d = tempfile::tempdir().unwrap();
        let c = check_codex_config(d.path());
        assert_eq!(c.level, Level::Fail);
        assert!(c.detail.contains("config.toml"));
        assert!(c.remedy.unwrap().contains("studio-cli doctor --fix"));
    }

    #[test]
    fn malformed_toml_is_treated_as_incomplete_not_a_crash() {
        let d = tempfile::tempdir().unwrap();
        write_codex_config(d.path(), "这不是合法的 toml {{{");
        let c = check_codex_config(d.path());
        assert_eq!(c.level, Level::Fail);
        assert_eq!(c.name, "作品的 Codex 配置不完整");
    }

    #[test]
    fn missing_command_key_is_incomplete() {
        let d = tempfile::tempdir().unwrap();
        write_codex_config(d.path(), "[mcp_servers.video-studio]\nother = \"x\"\n");
        let c = check_codex_config(d.path());
        assert_eq!(c.level, Level::Fail);
        assert_eq!(c.detail, "找不到 command 行");
    }

    #[test]
    fn stale_path_is_a_fail_that_names_the_broken_path() {
        let d = tempfile::tempdir().unwrap();
        fix_codex_config(d.path(), "/definitely/not/a/real/path/studiod").unwrap();
        let c = check_codex_config(d.path());
        assert_eq!(c.level, Level::Fail);
        assert_eq!(c.name, "作品指向的程序不存在");
        assert!(c.detail.contains("/definitely/not/a/real/path/studiod"));
        assert!(c.remedy.unwrap().contains("studio-cli doctor --fix"));
    }

    #[test]
    fn valid_path_passes() {
        let d = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap();
        fix_codex_config(d.path(), &exe.display().to_string()).unwrap();
        let c = check_codex_config(d.path());
        assert_eq!(c.level, Level::Ok);
        assert!(c.remedy.is_none());
    }

    #[test]
    fn run_without_a_bundle_skips_the_codex_config_check() {
        let report = run(None, None);
        assert!(report.bundle.is_none());
        assert!(!report.checks.iter().any(|c| c.name.contains("Codex 配置")));
        // ffmpeg、ffprobe、ComfyUI 节点、卡片生成基线——不看 bundle 时总归只有这四项。
        assert_eq!(report.checks.len(), 4);
    }

    #[test]
    fn run_with_a_bundle_missing_codex_config_is_unhealthy() {
        let d = tempfile::tempdir().unwrap();
        let report = run(None, Some(d.path()));
        assert!(!report.healthy, "缺 .codex/config.toml 应该判不健康");
        assert!(report
            .checks
            .iter()
            .any(|c| c.name.contains("Codex 配置缺失")));
    }

    #[test]
    fn render_reports_the_worst_outcome_first() {
        let report = Report {
            healthy: false,
            program_dir: None,
            bundle: None,
            checks: vec![Check {
                name: "坏事".into(),
                level: Level::Fail,
                detail: "详情".into(),
                remedy: Some("修一下".into()),
            }],
            tools: vec![],
            nodes: vec![],
        };
        let text = render(&report);
        assert!(text.contains("有必须先解决的问题"));
        assert!(text.contains("坏事"));
        assert!(text.contains("修一下"));
    }

    #[test]
    fn render_distinguishes_warnings_from_a_clean_pass() {
        let warn_only = Report {
            healthy: true,
            program_dir: None,
            bundle: None,
            checks: vec![Check {
                name: "小问题".into(),
                level: Level::Warn,
                detail: "详情".into(),
                remedy: None,
            }],
            tools: vec![],
            nodes: vec![],
        };
        assert!(render(&warn_only).contains("现在就可以开始创作"));

        let clean = Report {
            healthy: true,
            program_dir: None,
            bundle: None,
            checks: vec![],
            tools: vec![],
            nodes: vec![],
        };
        assert!(render(&clean).contains("全流程就绪"));
    }
}
