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
    /// ComfyUI 入口的探活结果。只有一个——后端有几个节点是代理那一侧的事。
    pub node: NodeHealth,
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
    let node = comfy.health();
    if node.reachable {
        checks.push(Check {
            name: "ComfyUI".into(),
            level: Level::Ok,
            detail: format!("{} 可达，队列深度 {}", node.url, node.queue_depth),
            remedy: None,
        });
    } else {
        checks.push(Check {
            name: "ComfyUI 不可达".into(),
            level: Level::Warn,
            detail: format!(
                "{}{}",
                node.url,
                node.detail
                    .as_deref()
                    .map(|d| format!("：{d}"))
                    .unwrap_or_default()
            ),
            remedy: Some(
                "渲染之前必须能连上。在 .env 里配：\n    \
                 COMFY_NODE=https://主机名\n  \
                 需要鉴权的代理再配 COMFY_TOKEN=<token>。\n  \
                 本机不需要 GPU，ComfyUI 可以在另一台机器上。\n  \
                 提交给 ComfyUI 之前的六个阶段不受影响，现在就可以开始创作。"
                    .into(),
            ),
        });
    }

    // 旧的多节点写法（`COMFY_NODES` 环境变量，或 config.toml 的 `[comfy].nodes`）
    // 只用第一个。被忽略的那些必须说出来——静默丢掉配置正是这个项目最不能接受
    // 的失败方式。
    let extras = settings.comfy_node_legacy_extras();
    if !extras.is_empty() {
        checks.push(Check {
            name: "旧的多节点配置里多余的地址被忽略".into(),
            level: Level::Warn,
            detail: format!("只用了第一个；被忽略的：{}", extras.join("、")),
            remedy: Some(format!(
                "入口现在只有一个 URL——多节点的分发由那一侧的代理负责，\n  \
                 控制面不再维护节点集合。把 .env 改成：\n    \
                 COMFY_NODE={}\n  \
                 config.toml 里的 [comfy].nodes 同理，改成 node = \"…\" 一项。\n  \
                 需要并发多压几个镜头就配 COMFY_CONCURRENCY（默认 16）。",
                node.url
            )),
        });
    }

    checks.push(check_workflow_assets(program_dir));
    checks.push(check_image_baselines(program_dir));
    checks.push(check_upscale_baseline(program_dir));

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
        node,
    }
}

/// 基线目录在不在 `studiod` 旁边。
///
/// **这一项是 Fail，不是 Warn。** 它管的是本项目最怕的那种失败：
/// `studiod` 找不到 `assets/workflows/` 时，能力面是空的，于是提交
/// `prompt_pack` 时的对账**整个不跑**——写了基线不吃的参数、帧数不在网格上、
/// 引用了不存在的资产，一律照收。错误不会消失，只是从「提交那一刻」推迟到
/// 「烧完 GPU 之后」，而 SPEC-0014 §6 那套校验存在的全部理由就是不让它推迟。
///
/// 这不是假设出来的：一次真实的 Codex 端到端会话里，release 二进制旁边没有
/// assets，五个镜头的帧数全都不在 `17k+5` 网格上，照样提交通过了。
fn check_workflow_assets(program_dir: Option<&std::path::Path>) -> Check {
    let dir = program_dir
        .map(|p| p.join("assets/workflows"))
        .unwrap_or_else(|| std::path::PathBuf::from("assets/workflows"));
    let families: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            let mut v: Vec<String> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();

    if families.is_empty() {
        return Check {
            name: "基线目录不在 studiod 旁边".into(),
            level: Level::Fail,
            detail: format!("{} 下一个模型系列都没有", dir.display()),
            remedy: Some(format!(
                "提交 prompt_pack 时的能力面对账会**整个不跑**——参数写错、帧数不在网格上、\n  \
                 引用了不存在的资产，全都照收，等烧完 GPU 才发现。\n  \
                 把仓库的 assets/ 整个复制到 studiod 所在目录：\n    \
                 cp -r <仓库>/assets {}",
                program_dir
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<studiod 所在目录>".into())
            )),
        };
    }
    Check {
        name: "基线目录".into(),
        level: Level::Ok,
        detail: format!("{} 个系列：{}", families.len(), families.join("、")),
        remedy: None,
    }
}

/// 卡片能不能生成，取决于 `flux2_dev` 的两条基线在不在、核验过没有。
///
/// 这一项**永远不是 Fail**：视觉资产阶段可以只提交计划，前六个阶段更是
/// 完全不受影响。它存在的意义是把「计划能提交」和「图能生出来」分开说清楚，
/// 免得有人看着一份通过校验的资产计划，以为卡片已经有了。
fn check_image_baselines(program_dir: Option<&std::path::Path>) -> Check {
    let dir = program_dir
        .map(|p| p.join("assets/workflows/flux2_dev"))
        .unwrap_or_else(|| std::path::PathBuf::from("assets/workflows/flux2_dev"));

    let mut missing = Vec::new();
    let mut unverified = Vec::new();
    for name in ["t2i", "multiref_edit"] {
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
            detail: "flux2_dev 的 t2i 与 multiref_edit 都在，且已核验".into(),
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

/// 成片超分要一份已核验的 SeedVR2 基线。
///
/// 这一项是 **Warn**：超分默认开着，但关掉（`COMFY_UPSCALE=0`）之后
/// `post` 就是原来那条纯 ffmpeg 的路，前九个阶段一步都不少。它存在的意义
/// 是别等到跑完渲染、进了 `post` 才发现交付规格达不到——那时候 GPU 时间
/// 已经烧完了。
fn check_upscale_baseline(program_dir: Option<&std::path::Path>) -> Check {
    let path = program_dir
        .map(|p| p.join("assets/workflows/seedvr2/upscale.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("assets/workflows/seedvr2/upscale.json"));

    let state = std::fs::read_to_string(&path).ok().map(|text| {
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("_studio")?.get("bindings_verified")?.as_bool())
            .unwrap_or(false)
    });

    match state {
        Some(true) => Check {
            name: "成片超分基线".into(),
            level: Level::Ok,
            detail: "seedvr2/upscale 在，且已核验".into(),
            remedy: None,
        },
        Some(false) => Check {
            name: "成片超分基线未核验".into(),
            level: Level::Warn,
            detail: format!("{} 的 bindings_verified 是 false", path.display()),
            remedy: Some(
                "post 会在超分那一步结构化阻塞。要么在真机上跑通并核验，\n  \
                 要么 COMFY_UPSCALE=0 明确接受原生画布的成片。"
                    .into(),
            ),
        },
        None => Check {
            name: "成片超分基线缺失".into(),
            level: Level::Warn,
            detail: format!("{} 不存在", path.display()),
            remedy: Some(format!(
                "post 默认要超分到交付规格（短边 1080），基线不在就会在那一步阻塞。\n  \
                 把仓库的 assets/ 整个复制到 studiod 所在目录；\n  \
                 或者 COMFY_UPSCALE=0 明确接受原生画布的成片（{}）。",
                path.display()
            )),
        },
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
        let dir = tmp.path().join("assets/workflows/flux2_dev");
        baseline(&dir, "t2i", true);
        baseline(&dir, "multiref_edit", false);
        let c = check_image_baselines(Some(tmp.path()));
        assert_eq!(c.level, Level::Warn);
        assert!(c.detail.contains("multiref_edit 未核验"), "{}", c.detail);
    }

    #[test]
    fn both_verified_baselines_report_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("assets/workflows/flux2_dev");
        baseline(&dir, "t2i", true);
        baseline(&dir, "multiref_edit", true);
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

    /// studiod 旁边没有基线目录 = 提交时的能力面对账整个不跑。
    /// 这是 Fail 不是 Warn：错误不会消失，只是推迟到烧完 GPU 之后。
    #[test]
    fn missing_workflow_assets_is_a_failure_not_a_warning() {
        let d = tempfile::tempdir().unwrap();
        let c = check_workflow_assets(Some(d.path()));
        assert_eq!(c.level, Level::Fail);
        let remedy = c.remedy.expect("必须给出可执行的补救");
        assert!(remedy.contains("cp -r"), "要给出照抄就能跑的命令：{remedy}");
        assert!(remedy.contains("整个不跑"), "要说清后果：{remedy}");
    }

    #[test]
    fn present_workflow_assets_lists_the_families() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("assets/workflows/minimax_h3")).unwrap();
        std::fs::create_dir_all(d.path().join("assets/workflows/ltx2_5")).unwrap();
        let c = check_workflow_assets(Some(d.path()));
        assert_eq!(c.level, Level::Ok);
        assert!(c.detail.contains("minimax_h3") && c.detail.contains("ltx2_5"));
    }

    #[test]
    fn run_without_a_bundle_skips_the_codex_config_check() {
        let report = run(None, None);
        assert!(report.bundle.is_none());
        assert!(!report.checks.iter().any(|c| c.name.contains("Codex 配置")));
        // ffmpeg、ffprobe、ComfyUI 节点、基线目录、卡片生成基线、成片超分
        // 基线——不看 bundle 时总归只有这六项。
        assert_eq!(report.checks.len(), 6);
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
            node: NodeHealth {
                url: "http://127.0.0.1:9001".into(),
                reachable: false,
                queue_depth: usize::MAX,
                detail: None,
            },
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
            node: NodeHealth {
                url: "http://127.0.0.1:9001".into(),
                reachable: false,
                queue_depth: usize::MAX,
                detail: None,
            },
        };
        assert!(render(&warn_only).contains("现在就可以开始创作"));

        let clean = Report {
            healthy: true,
            program_dir: None,
            bundle: None,
            checks: vec![],
            tools: vec![],
            node: NodeHealth {
                url: "http://127.0.0.1:9001".into(),
                reachable: false,
                queue_depth: usize::MAX,
                detail: None,
            },
        };
        assert!(render(&clean).contains("全流程就绪"));
    }
}
