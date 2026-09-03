//! 把端到端报告渲染成单文件 HTML。
//!
//! 只有一个外部依赖：Tailwind 的 CDN 脚本。其余全部内联，
//! 拷到哪儿都能打开，不需要起服务。

use crate::e2e::{human_ms, Report};

pub fn render(r: &Report) -> String {
    let mut h = String::with_capacity(32 * 1024);
    h.push_str(HEAD);

    let verdict_ok = r.passed;
    h.push_str(&format!(
        r#"<body class="bg-slate-50 text-slate-800 dark:bg-slate-950 dark:text-slate-200">
<div class="mx-auto max-w-6xl px-6 py-10">

<header class="mb-8 border-b border-slate-200 pb-6 dark:border-slate-800">
  <div class="flex flex-wrap items-baseline justify-between gap-3">
    <div>
      <p class="font-mono text-xs uppercase tracking-widest text-teal-700 dark:text-teal-400">端到端报告</p>
      <h1 class="mt-1 text-2xl font-semibold tracking-tight">{title}</h1>
    </div>
    <span class="rounded px-3 py-1 text-sm font-semibold {badge}">{verdict}</span>
  </div>
  <dl class="mt-4 grid gap-x-8 gap-y-1 font-mono text-xs text-slate-500 dark:text-slate-400 sm:grid-cols-2">
    <div><dt class="inline">作品目录 </dt><dd class="inline text-slate-700 dark:text-slate-300">{bundle}</dd></div>
    <div><dt class="inline">生成时间 </dt><dd class="inline text-slate-700 dark:text-slate-300">{at}</dd></div>
  </dl>
</header>
"#,
        title = esc(&project_title(r)),
        bundle = esc(&r.bundle),
        at = esc(&r.generated_at),
        badge = if verdict_ok {
            "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300"
        } else {
            "bg-rose-100 text-rose-800 dark:bg-rose-900/50 dark:text-rose-300"
        },
        verdict = if verdict_ok { "通过" } else { "未通过" },
    ));

    // ---- 关键指标 ----
    h.push_str(r#"<section class="mb-10"><div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">"#);
    let tokens = r.rollout.as_ref().map(|ro| ro.tokens.clone());
    let metrics: Vec<(&str, String, &str)> = vec![
        (
            "有效耗时",
            human_ms(r.timing.effective_ms),
            "不含等待用户确认",
        ),
        ("墙上耗时", human_ms(r.timing.wall_ms), "首次调用到最后一次"),
        (
            "等待用户",
            human_ms(r.timing.waiting_user_ms),
            "挂在确认门上",
        ),
        (
            "MCP 调用",
            r.total_calls.to_string(),
            if r.failed_calls == 0 {
                "全部成功"
            } else {
                "见下方阻塞"
            },
        ),
        (
            "输入 token",
            tokens
                .as_ref()
                .map(|t| num(t.input))
                .unwrap_or_else(|| "—".into()),
            match &tokens {
                Some(t) => Box::leak(format!("命中缓存 {}", num(t.cached_input)).into_boxed_str()),
                None => "需要 --rollout",
            },
        ),
        (
            "输出 token",
            tokens
                .as_ref()
                .map(|t| num(t.output))
                .unwrap_or_else(|| "—".into()),
            match &tokens {
                Some(t) => Box::leak(format!("推理 {}", num(t.reasoning_output)).into_boxed_str()),
                None => "需要 --rollout",
            },
        ),
    ];
    for (label, value, note) in metrics {
        h.push_str(&format!(
            r#"<div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
  <p class="font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:text-slate-400">{label}</p>
  <p class="mt-1 text-xl font-semibold tabular-nums">{value}</p>
  <p class="mt-0.5 text-xs text-slate-400 dark:text-slate-500">{note}</p>
</div>"#,
            label = esc(label),
            value = esc(&value),
            note = esc(note)
        ));
    }
    h.push_str("</div></section>");

    // ---- 验收 ----
    h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">验收</h2><ul class="space-y-2">"#);
    for v in &r.verdicts {
        h.push_str(&format!(
            r#"<li class="flex gap-3 rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-900">
  <span class="mt-0.5 shrink-0 rounded px-2 py-0.5 font-mono text-[11px] {cls}">{mark}</span>
  <div><p class="font-medium">{name}</p><p class="mt-0.5 text-sm text-slate-500 dark:text-slate-400">{detail}</p></div>
</li>"#,
            cls = if v.passed {
                "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300"
            } else {
                "bg-rose-100 text-rose-800 dark:bg-rose-900/50 dark:text-rose-300"
            },
            mark = if v.passed { "通过" } else { "未过" },
            name = esc(&v.name),
            detail = esc(&v.detail)
        ));
    }
    h.push_str("</ul></section>");

    // ---- 耗时拆解 ----
    let t = &r.timing;
    let denom = (t.server_ms + t.agent_ms + t.waiting_user_ms).max(1);
    h.push_str(&format!(
        r#"<section class="mb-10">
<h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">时间去哪了</h2>
<div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
  <div class="flex h-6 overflow-hidden rounded" role="img" aria-label="耗时构成">
    <div class="bg-teal-600" style="width:{p1:.2}%" title="控制面 {v1}"></div>
    <div class="bg-sky-500" style="width:{p2:.2}%" title="Agent {v2}"></div>
    <div class="bg-slate-300 dark:bg-slate-700" style="width:{p3:.2}%" title="等待用户 {v3}"></div>
  </div>
  <div class="mt-3 grid gap-2 text-sm sm:grid-cols-3">
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-teal-600"></span>控制面处理 <span class="font-mono tabular-nums">{v1}</span></p>
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-sky-500"></span>Agent 生成 <span class="font-mono tabular-nums">{v2}</span></p>
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-slate-300 dark:bg-slate-700"></span>等待用户确认 <span class="font-mono tabular-nums">{v3}</span></p>
  </div>
  <p class="mt-3 text-xs text-slate-400 dark:text-slate-500">等待用户确认不计入有效耗时——那是人在看，不是系统在跑。</p>
</div>
</section>"#,
        p1 = t.server_ms as f64 * 100.0 / denom as f64,
        p2 = t.agent_ms as f64 * 100.0 / denom as f64,
        p3 = t.waiting_user_ms as f64 * 100.0 / denom as f64,
        v1 = human_ms(t.server_ms),
        v2 = human_ms(t.agent_ms),
        v3 = human_ms(t.waiting_user_ms),
    ));

    // ---- Skill 维度 ----
    if !r.skills.is_empty() {
        h.push_str(r#"<section class="mb-10">
<h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">各 Skill</h2>
<div class="overflow-x-auto rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900">
<table class="w-full text-sm"><thead class="border-b border-slate-200 text-left font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:border-slate-800 dark:text-slate-400">
<tr><th class="p-3">Skill</th><th class="p-3">阶段</th><th class="p-3 text-right">调用</th><th class="p-3 text-right">控制面</th><th class="p-3 text-right">Agent</th><th class="p-3 text-right">等用户</th></tr>
</thead><tbody>"#);
        for s in &r.skills {
            h.push_str(&format!(
                r#"<tr class="border-b border-slate-100 last:border-0 dark:border-slate-800">
<td class="p-3 font-medium">{cap}</td><td class="p-3 font-mono text-xs text-slate-500 dark:text-slate-400">{stages}</td>
<td class="p-3 text-right tabular-nums">{calls}</td><td class="p-3 text-right tabular-nums">{sm}</td>
<td class="p-3 text-right tabular-nums">{am}</td><td class="p-3 text-right tabular-nums text-slate-400">{wm}</td></tr>"#,
                cap = esc(&s.capability),
                stages = esc(&s.stages.join(" · ")),
                calls = s.calls,
                sm = esc(&human_ms(s.server_ms)),
                am = esc(&human_ms(s.agent_ms)),
                wm = esc(&human_ms(s.waiting_user_ms)),
            ));
        }
        h.push_str("</tbody></table></div></section>");
    }

    // ---- 工具调用 ----
    h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">MCP 工具调用</h2><div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">"#);
    let max = r.calls_by_tool.values().copied().max().unwrap_or(1).max(1);
    for (tool, n) in &r.calls_by_tool {
        h.push_str(&format!(
            r#"<div class="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-900">
  <div class="flex items-baseline justify-between"><span class="font-mono text-xs">{tool}</span><span class="font-semibold tabular-nums">{n}</span></div>
  <div class="mt-2 h-1.5 rounded bg-slate-100 dark:bg-slate-800"><div class="h-1.5 rounded bg-teal-600" style="width:{pct:.1}%"></div></div>
</div>"#,
            tool = esc(tool),
            n = n,
            pct = *n as f64 * 100.0 / max as f64
        ));
    }
    h.push_str("</div></section>");

    // ---- 阻塞 ----
    if !r.errors.is_empty() {
        h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">遇到的阻塞</h2>
<div class="overflow-x-auto rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900"><table class="w-full text-sm">
<thead class="border-b border-slate-200 text-left font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:border-slate-800 dark:text-slate-400">
<tr><th class="p-3">时刻</th><th class="p-3">工具</th><th class="p-3">阶段</th><th class="p-3">错误码</th><th class="p-3">补救路径</th></tr></thead><tbody>"#);
        for e in &r.errors {
            h.push_str(&format!(
                r#"<tr class="border-b border-slate-100 last:border-0 dark:border-slate-800">
<td class="p-3 font-mono text-xs text-slate-500">{at}</td><td class="p-3 font-mono text-xs">{tool}</td>
<td class="p-3 font-mono text-xs">{stage}</td><td class="p-3"><code class="rounded bg-amber-100 px-1.5 py-0.5 text-xs text-amber-900 dark:bg-amber-900/40 dark:text-amber-200">{code}</code></td>
<td class="p-3 text-xs">{remedy}</td></tr>"#,
                at = esc(&e.at),
                tool = esc(&e.tool),
                stage = esc(e.stage.as_deref().unwrap_or("—")),
                code = esc(&e.code),
                remedy = if e.remedy_present {
                    r#"<span class="text-emerald-700 dark:text-emerald-400">有</span>"#
                } else {
                    r#"<span class="font-semibold text-rose-700 dark:text-rose-400">缺失 —— 这是实现缺陷</span>"#
                }
            ));
        }
        h.push_str("</tbody></table></div></section>");
    }

    // ---- Codex 侧 ----
    match &r.rollout {
        Some(ro) => {
            h.push_str(&format!(
                r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">Codex 侧</h2>
<div class="grid gap-3 lg:grid-cols-2">
  <div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
    <p class="mb-2 text-sm font-medium">会话</p>
    <dl class="space-y-1 font-mono text-xs text-slate-600 dark:text-slate-400">
      <div><dt class="inline">模型 </dt><dd class="inline">{model}</dd></div>
      <div><dt class="inline">用户消息 </dt><dd class="inline tabular-nums">{um}</dd></div>
      <div><dt class="inline">助手消息 </dt><dd class="inline tabular-nums">{am}</dd></div>
      <div><dt class="inline">推理块 </dt><dd class="inline tabular-nums">{rb}</dd></div>
      <div><dt class="inline">上下文窗口 </dt><dd class="inline tabular-nums">{cw}</dd></div>
    </dl>
  </div>
  <div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
    <p class="mb-2 text-sm font-medium">Codex 发起的调用</p>
    <dl class="space-y-1 font-mono text-xs text-slate-600 dark:text-slate-400">
      <div><dt class="inline">本项目 MCP </dt><dd class="inline tabular-nums">{mcp}</dd></div>
      <div><dt class="inline">其它 MCP </dt><dd class="inline tabular-nums">{omcp}</dd></div>
      <div><dt class="inline">本地命令 </dt><dd class="inline tabular-nums">{sh}</dd></div>
      <div><dt class="inline">联网 </dt><dd class="inline tabular-nums">{web}</dd></div>
    </dl>
  </div>
</div>
<div class="mt-3 grid gap-3 lg:grid-cols-2">
  <div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
    <p class="mb-2 text-sm font-medium">读过的 Skill</p>
    <p class="text-sm text-slate-600 dark:text-slate-400">{skills}</p>
  </div>
  <div class="rounded-lg border p-4 {bcls}">
    <p class="mb-2 text-sm font-medium">是否绕过 MCP</p>
    <p class="text-sm">{bypass}</p>
  </div>
</div>
</section>"#,
                model = esc(ro.model.as_deref().unwrap_or("—")),
                um = ro.user_messages,
                am = ro.assistant_messages,
                rb = ro.reasoning_blocks,
                cw = ro.tokens.context_window.map(num).unwrap_or_else(|| "—".into()),
                mcp = ro.calls.studio_mcp,
                omcp = ro.calls.other_mcp,
                sh = ro.calls.shell,
                web = ro.calls.web,
                skills = if ro.skills_read.is_empty() {
                    "会话里没有读取 SKILL.md 的记录。".to_string()
                } else {
                    esc(&ro.skills_read.join("、"))
                },
                bcls = if ro.bypasses.is_empty() {
                    "border-emerald-200 bg-emerald-50 dark:border-emerald-900 dark:bg-emerald-950/40"
                } else {
                    "border-rose-200 bg-rose-50 dark:border-rose-900 dark:bg-rose-950/40"
                },
                bypass = if ro.bypasses.is_empty() {
                    "没有。状态变更全部经由 MCP。".to_string()
                } else {
                    esc(&ro.bypasses.join("；"))
                },
            ));
        }
        None => h.push_str(
            r#"<section class="mb-10"><div class="rounded-lg border border-dashed border-slate-300 p-4 text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
token 用量、读过哪些 Skill、有没有绕过 MCP 直接跑命令——这些只有 Codex 自己的会话记录里有，
MCP server 看不见。加 <code class="rounded bg-slate-100 px-1 dark:bg-slate-800">--rollout &lt;会话.jsonl&gt;</code> 合并进来。
</div></section>"#,
        ),
    }

    h.push_str(&format!(
        r#"<footer class="border-t border-slate-200 pt-6 text-xs text-slate-400 dark:border-slate-800 dark:text-slate-500">
video-studio {ver} · 数据来自作品的 .studio/trace.jsonl{extra}
</footer>
</div></body>"#,
        ver = env!("CARGO_PKG_VERSION"),
        extra = match &r.rollout {
            Some(ro) => format!("，以及 {}", esc(&ro.source)),
            None => String::new(),
        }
    ));
    h
}

fn project_title(r: &Report) -> String {
    std::path::Path::new(&r.bundle)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| r.bundle.clone())
}

fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const HEAD: &str = r#"<!doctype html>
<html lang="zh-CN" class="antialiased">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>video-studio 端到端报告</title>
<script src="https://cdn.tailwindcss.com"></script>
<style>
  @media (prefers-color-scheme: dark) { html { color-scheme: dark; } }
  body { font-family: ui-sans-serif, system-ui, "PingFang SC", "Microsoft YaHei", sans-serif; }
  code, .font-mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
</style>
</head>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e;
    use studio_mcp::trace::{Trace, TraceRecord};

    fn sample_bundle() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let t = Trace::at(d.path());
        for (i, (tool, stage, waiting)) in [
            ("studio.submit_stage", "idea", "agent"),
            ("studio.submit_stage", "selection", "user"),
            ("studio.answer", "selection", "agent"),
        ]
        .iter()
        .enumerate()
        {
            t.append(&TraceRecord {
                at: format!("2026-09-03T00:0{i}:10.000Z"),
                tool: (*tool).into(),
                stage: Some((*stage).into()),
                capability: studio_core::StageId::parse(stage)
                    .map(|s| s.capability().as_str().into()),
                ok: true,
                error_code: None,
                remedy_present: None,
                waiting_on: Some((*waiting).into()),
                duration_ms: 12,
            });
        }
        d
    }

    #[test]
    fn html_is_self_contained_apart_from_tailwind() {
        let d = sample_bundle();
        let html = render(&e2e::build(d.path()));
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("cdn.tailwindcss.com"));
        // 除 Tailwind 之外不该再拉别的外部资源
        assert_eq!(html.matches("src=\"http").count(), 1);
        assert_eq!(html.matches("href=\"http").count(), 0);
        assert!(html.ends_with("</div></body>"));
    }

    #[test]
    fn waiting_for_the_user_is_excluded_from_effective_time() {
        let d = sample_bundle();
        let r = e2e::build(d.path());
        assert!(
            r.timing.waiting_user_ms > 0,
            "第二次调用之后在等用户，那段要算进等待"
        );
        assert_eq!(
            r.timing.effective_ms,
            r.timing.wall_ms - r.timing.waiting_user_ms
        );
        let html = render(&r);
        assert!(html.contains("等待用户确认不计入有效耗时"));
    }

    #[test]
    fn skills_are_summarised_by_capability() {
        let d = sample_bundle();
        let r = e2e::build(d.path());
        let caps: Vec<&str> = r.skills.iter().map(|s| s.capability.as_str()).collect();
        assert!(caps.contains(&"idea") && caps.contains(&"selection"));
        assert!(render(&r).contains("各 Skill"));
    }

    #[test]
    fn without_a_rollout_the_unobservable_columns_say_so() {
        let d = sample_bundle();
        let html = render(&e2e::build(d.path()));
        assert!(
            html.contains("--rollout"),
            "没有 rollout 时要说明 token 为什么是空的"
        );
        assert!(html.contains("MCP server 看不见"));
    }

    #[test]
    fn html_escapes_titles_and_paths() {
        let d = sample_bundle();
        let mut r = e2e::build(d.path());
        r.bundle = "/tmp/<script>alert(1)</script>.studio".into();
        let html = render(&r);
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(num(0), "0");
        assert_eq!(num(999), "999");
        assert_eq!(num(1000), "1,000");
        assert_eq!(num(1234567), "1,234,567");
    }
}
