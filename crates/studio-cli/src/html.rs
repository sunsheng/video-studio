//! 把端到端报告渲染成单文件 HTML。
//!
//! 只有一个外部依赖：Tailwind 的 CDN 脚本。其余全部内联，
//! 拷到哪儿都能打开，不需要起服务。

use crate::e2e::{human_ms, Report};
use crate::exec_report::Report as ExecReport;

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

/// 执行侧的 HTML。跟 Agent 侧是两份独立的报告，因为读者不同：
/// 那份看协作，这份看吞吐。
pub fn render_exec(r: &ExecReport) -> String {
    let mut h = String::with_capacity(16 * 1024);
    h.push_str(&HEAD.replace("video-studio 端到端报告", "video-studio 执行侧报告"));
    h.push_str(&format!(
        r#"<body class="bg-slate-50 text-slate-800 dark:bg-slate-950 dark:text-slate-200">
<div class="mx-auto max-w-6xl px-6 py-10">
<header class="mb-8 border-b border-slate-200 pb-6 dark:border-slate-800">
  <div class="flex flex-wrap items-baseline justify-between gap-3">
    <div>
      <p class="font-mono text-xs uppercase tracking-widest text-indigo-700 dark:text-indigo-400">执行侧报告 · ComfyUI 与后期</p>
      <h1 class="mt-1 text-2xl font-semibold tracking-tight">{title}</h1>
    </div>
    <span class="rounded px-3 py-1 text-sm font-semibold {badge}">{verdict}</span>
  </div>
  <p class="mt-3 text-sm text-slate-500 dark:text-slate-400">
    这份看吞吐：镜头排在哪个节点、GPU 等了多久、后期哪一步慢。
    Agent 那一侧（阶段推进、确认门、修订、token）是另一份独立报告。
  </p>
</header>
"#,
        title = esc(&title_of(&r.bundle)),
        badge = if !r.has_data {
            "bg-slate-200 text-slate-700 dark:bg-slate-800 dark:text-slate-300"
        } else if r.passed {
            "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300"
        } else {
            "bg-rose-100 text-rose-800 dark:bg-rose-900/50 dark:text-rose-300"
        },
        verdict = if !r.has_data {
            "尚未执行"
        } else if r.passed {
            "全部成功"
        } else {
            "有失败"
        },
    ));

    if !r.has_data {
        h.push_str(
            r#"<div class="rounded-lg border border-dashed border-slate-300 p-6 text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
这部作品还没跑过确定性阶段（渲染 / 后期 / 验收）。提示词包确认之后控制面会自动开始，跑完再来看这份报告。
</div></div></body>"#,
        );
        return h;
    }

    // 耗时构成
    let denom = r.total_ms.max(1);
    h.push_str(&format!(
        r#"<section class="mb-10">
<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
  {c1}{c2}{c3}{c4}
</div>
<div class="mt-4 rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
  <div class="flex h-6 overflow-hidden rounded">
    <div class="bg-indigo-600" style="width:{p1:.2}%" title="渲染"></div>
    <div class="bg-violet-500" style="width:{p2:.2}%" title="后期"></div>
    <div class="bg-slate-300 dark:bg-slate-700" style="width:{p3:.2}%" title="验收"></div>
  </div>
  <div class="mt-3 grid gap-2 text-sm sm:grid-cols-3">
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-indigo-600"></span>渲染（GPU）<span class="ml-1 font-mono tabular-nums">{v1}</span></p>
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-violet-500"></span>后期（ffmpeg）<span class="ml-1 font-mono tabular-nums">{v2}</span></p>
    <p><span class="mr-2 inline-block h-2 w-2 rounded-full bg-slate-300 dark:bg-slate-700"></span>验收（ffprobe）<span class="ml-1 font-mono tabular-nums">{v3}</span></p>
  </div>
</div>
</section>"#,
        c1 = metric("执行总耗时", &human_ms(r.total_ms as i64), "三个确定性阶段之和"),
        c2 = metric("渲染", &human_ms(r.render_ms as i64), "通常是大头"),
        c3 = metric("镜头数", &r.shots.len().to_string(), "逐镜头明细见下"),
        c4 = metric("用到节点", &r.nodes_used.to_string(), "并行度"),
        p1 = r.render_ms as f64 * 100.0 / denom as f64,
        p2 = r.post_ms as f64 * 100.0 / denom as f64,
        p3 = r.review_ms as f64 * 100.0 / denom as f64,
        v1 = human_ms(r.render_ms as i64),
        v2 = human_ms(r.post_ms as i64),
        v3 = human_ms(r.review_ms as i64),
    ));

    // 逐镜头
    if !r.shots.is_empty() {
        let slowest = r.shots.iter().map(|s| s.total_ms).max().unwrap_or(1).max(1);
        h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">逐镜头</h2>
<div class="overflow-x-auto rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900"><table class="w-full text-sm">
<thead class="border-b border-slate-200 text-left font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:border-slate-800 dark:text-slate-400">
<tr><th class="p-3">镜头</th><th class="p-3">节点</th><th class="p-3 text-right">选节点</th><th class="p-3 text-right">提交</th>
<th class="p-3 text-right">渲染</th><th class="p-3 text-right">下载</th><th class="p-3">占比</th></tr></thead><tbody>"#);
        for s in &r.shots {
            h.push_str(&format!(
                r#"<tr class="border-b border-slate-100 last:border-0 dark:border-slate-800">
<td class="p-3 font-mono font-medium">{id}{flag}</td>
<td class="p-3 font-mono text-xs text-slate-500 dark:text-slate-400">{node}</td>
<td class="p-3 text-right tabular-nums text-slate-500">{pick}</td>
<td class="p-3 text-right tabular-nums text-slate-500">{submit}</td>
<td class="p-3 text-right font-medium tabular-nums">{render}</td>
<td class="p-3 text-right tabular-nums text-slate-500">{dl}</td>
<td class="p-3 w-40"><div class="h-1.5 rounded bg-slate-100 dark:bg-slate-800"><div class="h-1.5 rounded bg-indigo-600" style="width:{pct:.1}%"></div></div></td></tr>"#,
                id = esc(&s.shot_id),
                flag = if s.ok {
                    String::new()
                } else {
                    format!(
                        r#" <span class="rounded bg-rose-100 px-1 text-[10px] text-rose-800 dark:bg-rose-900/50 dark:text-rose-300">{}</span>"#,
                        esc(s.error_code.as_deref().unwrap_or("失败"))
                    )
                },
                node = esc(s.node.as_deref().unwrap_or("—")),
                pick = esc(&human_ms(s.pick_ms as i64)),
                submit = esc(&human_ms(s.submit_ms as i64)),
                render = esc(&human_ms(s.render_ms as i64)),
                dl = esc(&human_ms(s.download_ms as i64)),
                pct = s.total_ms as f64 * 100.0 / slowest as f64,
            ));
        }
        h.push_str("</tbody></table></div></section>");
    }

    // 节点负载
    if !r.nodes.is_empty() {
        let busiest = r
            .nodes
            .iter()
            .map(|n| n.render_ms)
            .max()
            .unwrap_or(1)
            .max(1);
        h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">节点负载</h2><div class="grid gap-2 sm:grid-cols-2">"#);
        for n in &r.nodes {
            h.push_str(&format!(
                r#"<div class="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-900">
<div class="flex items-baseline justify-between"><span class="font-mono text-xs">{node}</span>
<span class="text-sm"><span class="font-semibold tabular-nums">{shots}</span> 个镜头 · <span class="tabular-nums">{t}</span></span></div>
<div class="mt-2 h-1.5 rounded bg-slate-100 dark:bg-slate-800"><div class="h-1.5 rounded bg-indigo-600" style="width:{pct:.1}%"></div></div></div>"#,
                node = esc(&n.node),
                shots = n.shots,
                t = esc(&human_ms(n.render_ms as i64)),
                pct = n.render_ms as f64 * 100.0 / busiest as f64
            ));
        }
        h.push_str("</div></section>");
    }

    // 步骤
    h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">各步骤</h2>
<div class="overflow-x-auto rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900"><table class="w-full text-sm">
<thead class="border-b border-slate-200 text-left font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:border-slate-800 dark:text-slate-400">
<tr><th class="p-3">阶段</th><th class="p-3">步骤</th><th class="p-3 text-right">次数</th><th class="p-3 text-right">耗时</th><th class="p-3">详情</th></tr></thead><tbody>"#);
    for st in &r.steps {
        h.push_str(&format!(
            r#"<tr class="border-b border-slate-100 last:border-0 dark:border-slate-800">
<td class="p-3 font-mono text-xs">{stage}</td><td class="p-3 font-mono text-xs {cls}">{step}</td>
<td class="p-3 text-right tabular-nums">{calls}</td><td class="p-3 text-right tabular-nums">{t}</td>
<td class="p-3 font-mono text-xs text-slate-500 dark:text-slate-400">{detail}</td></tr>"#,
            stage = esc(&st.stage),
            step = esc(&st.step),
            cls = if st.ok {
                ""
            } else {
                "text-rose-700 dark:text-rose-400"
            },
            calls = st.calls,
            t = esc(&human_ms(st.total_ms as i64)),
            detail = esc(st.detail.as_deref().unwrap_or("")),
        ));
    }
    h.push_str("</tbody></table></div></section>");

    if !r.failures.is_empty() {
        h.push_str(r#"<section class="mb-10"><h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400">失败</h2><ul class="space-y-2">"#);
        for f in &r.failures {
            h.push_str(&format!(
                r#"<li class="rounded-lg border border-rose-200 bg-rose-50 p-3 text-sm dark:border-rose-900 dark:bg-rose-950/40">
<span class="font-mono text-xs text-slate-500">{at}</span>
<span class="ml-2 font-mono">{stage}/{step}</span>
<span class="ml-2 font-mono text-xs">{shot}</span>
<code class="ml-2 rounded bg-rose-100 px-1.5 py-0.5 text-xs text-rose-900 dark:bg-rose-900/60 dark:text-rose-200">{code}</code></li>"#,
                at = esc(&f.at),
                stage = esc(&f.stage),
                step = esc(&f.step),
                shot = esc(f.shot_id.as_deref().unwrap_or("")),
                code = esc(f.error_code.as_deref().unwrap_or("未知")),
            ));
        }
        h.push_str("</ul></section>");
    }

    h.push_str(&format!(
        r#"<footer class="border-t border-slate-200 pt-6 text-xs text-slate-400 dark:border-slate-800 dark:text-slate-500">
video-studio {ver} · 数据来自作品的 .studio/exec.jsonl · Agent 侧报告见 studiod e2e report
</footer></div></body>"#,
        ver = env!("CARGO_PKG_VERSION")
    ));
    h
}

fn metric(label: &str, value: &str, note: &str) -> String {
    format!(
        r#"<div class="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
  <p class="font-mono text-[11px] uppercase tracking-wider text-slate-500 dark:text-slate-400">{}</p>
  <p class="mt-1 text-xl font-semibold tabular-nums">{}</p>
  <p class="mt-0.5 text-xs text-slate-400 dark:text-slate-500">{}</p>
</div>"#,
        esc(label),
        esc(value),
        esc(note)
    )
}

fn title_of(bundle: &str) -> String {
    std::path::Path::new(bundle)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| bundle.to_string())
}

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
    fn the_exec_report_is_a_separate_page_that_points_at_the_other_one() {
        let d = tempfile::tempdir().unwrap();
        let r = crate::exec_report::build(d.path());
        let h = render_exec(&r);
        assert!(h.starts_with("<!doctype html>"));
        assert!(h.contains("执行侧报告"));
        assert!(h.contains("尚未执行"));
        assert!(h.contains("另一份独立报告"), "两份报告要互相指认");
        assert_eq!(h.matches("src=\"http").count(), 1);
    }

    #[test]
    fn exec_html_shows_per_shot_and_node_load() {
        let d = tempfile::tempdir().unwrap();
        let rec = studio_engine::ExecRecorder::at(d.path());
        for (shot, node, ms) in [
            ("sh01", "http://n1:9001", 40_000u64),
            ("sh02", "http://n2:9002", 25_000),
        ] {
            rec.append(&studio_engine::ExecRecord {
                at: "2026-09-03T00:00:00.000Z".into(),
                stage: "render".into(),
                step: "render".into(),
                shot_id: Some(shot.into()),
                node: Some(node.into()),
                prompt_id: Some(format!("p-{shot}")),
                duration_ms: ms,
                ok: true,
                error_code: None,
                extra: serde_json::Map::new(),
            });
        }
        let h = render_exec(&crate::exec_report::build(d.path()));
        assert!(h.contains("sh01") && h.contains("sh02"));
        assert!(h.contains("http://n1:9001"));
        assert!(h.contains("节点负载"));
        assert!(h.contains("全部成功"));
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(num(0), "0");
        assert_eq!(num(999), "999");
        assert_eq!(num(1000), "1,000");
        assert_eq!(num(1234567), "1,234,567");
    }
}
