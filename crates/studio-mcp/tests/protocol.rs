//! MCP 协议一致性：完整走一遍 JSON-RPC，不经过任何内部捷径。
//!
//! 这些用例覆盖「提交给 ComfyUI 之前」的全部六个阶段，
//! 不需要 GPU、ComfyUI 或 ffmpeg，因此在开发环境就能跑。

use serde_json::{json, Value};
use studio_core::{fixtures, StageId};
use studio_mcp::{trace::Trace, Server};

struct Harness {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    server: Server,
    next_id: i64,
}

impl Harness {
    fn new() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("千岛湖.studio");
        studio_engine::init_project(&root, fixtures::TITLE, "0.1.0-test", &[]).unwrap();
        let server = Server::new(&root, None);
        Harness {
            _dir: dir,
            root,
            server,
            next_id: 0,
        }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let req =
            json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        let raw = self
            .server
            .handle_line(&req.to_string())
            .expect("请求必须有回应");
        serde_json::from_str(&raw).unwrap()
    }

    /// 调用一个工具，返回 (结构化载荷, 是否报错)。
    fn call(&mut self, name: &str, args: Value) -> (Value, bool) {
        let resp = self.rpc("tools/call", json!({ "name": name, "arguments": args }));
        let result = &resp["result"];
        (
            result["structuredContent"].clone(),
            result["isError"].as_bool().unwrap_or(false),
        )
    }

    fn submit(&mut self, stage: StageId) -> (Value, bool) {
        let mut args = json!({
            "outputs": fixtures::outputs(stage),
            "summary": fixtures::summary(stage),
        });
        if let Some(c) = fixtures::confirmation(stage) {
            args["confirmation"] = serde_json::to_value(c).unwrap();
        }
        self.call("studio.submit_stage", args)
    }

    /// 提交并在有门时确认通过。
    fn advance(&mut self, stage: StageId) {
        let (env, err) = self.submit(stage);
        assert!(!err, "提交 {stage} 失败：{env}");
        if let Some(q) = env["pending_question"].as_object() {
            let qid = q["question_id"].as_str().unwrap().to_string();
            // 不写死 "approve"：选题那道门的通过选项是几个方案各一个（id 是 concept_id）。
            let answer = q["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["outcome"] == "approve")
                .unwrap_or_else(|| panic!("{stage} 的门上没有通过选项"))["id"]
                .as_str()
                .unwrap()
                .to_string();
            let (_, err) = self.call(
                "studio.answer",
                json!({ "question_id": qid, "answer": answer }),
            );
            assert!(!err, "确认 {stage} 失败");
        }
    }
}

#[test]
fn initialize_echoes_a_protocol_version_we_support() {
    let mut h = Harness::new();
    let resp = h.rpc(
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {} }),
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(resp["result"]["serverInfo"]["name"], "video-studio");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn initialize_falls_back_for_an_unknown_version() {
    let mut h = Harness::new();
    let resp = h.rpc("initialize", json!({ "protocolVersion": "1999-01-01" }));
    assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn tools_list_exposes_exactly_eleven_tools_without_run_id() {
    let mut h = Harness::new();
    let resp = h.rpc("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 11);
    for t in tools {
        let name = t["name"].as_str().unwrap();
        assert!(name.starts_with("studio."));
        assert!(t["description"].as_str().unwrap().chars().count() > 12);
        let props = &t["inputSchema"]["properties"];
        assert!(props.get("run_id").is_none(), "{name} 不该有 run_id");
    }
}

#[test]
fn notifications_get_no_response() {
    let mut h = Harness::new();
    let raw = h.server.handle_line(
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
    );
    assert!(raw.is_none(), "通知不该有回应");
}

#[test]
fn malformed_json_is_reported_not_panicked() {
    let mut h = Harness::new();
    let raw = h.server.handle_line("{ 这不是 JSON").unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["error"]["code"], -32700);
}

#[test]
fn unknown_method_and_unknown_tool_are_rejected_clearly() {
    let mut h = Harness::new();
    let resp = h.rpc("does/not/exist", json!({}));
    assert_eq!(resp["error"]["code"], -32601);

    let resp = h.rpc(
        "tools/call",
        json!({ "name": "studio.launch_missiles", "arguments": {} }),
    );
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("tools/list"));
}

#[test]
fn status_tells_the_agent_what_to_do_first() {
    let mut h = Harness::new();
    let (env, err) = h.call("studio.status", json!({}));
    assert!(!err);
    assert_eq!(env["project"]["stage"], "idea");
    assert_eq!(env["waiting_on"], "agent");
    assert_eq!(env["next_action"]["kind"], "submit_stage");
    assert_eq!(env["next_action"]["schema_ref"], "idea");
    assert_eq!(env["progress"]["total"], 10);
}

#[test]
fn schema_is_available_before_submitting() {
    let mut h = Harness::new();
    let (doc, err) = h.call("studio.schema", json!({ "stage": "script" }));
    assert!(!err);
    assert_eq!(doc["required"][0], "script");
    let props = &doc["properties"]["script"]["properties"];
    assert!(props["story_arc"].is_object());
    assert!(props["segments"].is_object());
}

/// 提交 ComfyUI 之前的六个阶段，全程走 MCP。
#[test]
fn the_six_stages_before_comfyui_run_end_to_end_over_mcp() {
    let mut h = Harness::new();
    h.rpc("initialize", json!({ "protocolVersion": "2025-06-18" }));

    for s in [
        StageId::Idea,
        StageId::Selection,
        StageId::Script,
        StageId::Storyboard,
        StageId::VisualAssets,
        StageId::PromptPack,
    ] {
        h.advance(s);
    }

    let (env, _) = h.call("studio.status", json!({}));
    assert_eq!(env["progress"]["completed"], 6);
    assert_eq!(env["project"]["stage"], "preview");
    assert_eq!(
        env["waiting_on"], "system",
        "轮到 ComfyUI 了（先出便宜的预览），Agent 只需观察"
    );
    assert!(env["blocked_by"].is_null());

    // 提示词包已经带着可提交给 ComfyUI 的全部参数
    let (pack, _) = h.call("studio.stage_output", json!({ "stage": "prompt_pack" }));
    let shots = pack["prompt_pack"]["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 5);
    assert_eq!(shots[0]["workflow"], "minimax_h3/t2v");

    let (tl, _) = h.call("studio.timeline", json!({ "limit": 100 }));
    let kinds: Vec<&str> = tl
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.iter().filter(|k| **k == "submitted").count() >= 6);
}

/// 用户在门上说「不要固定 2 秒」——三次调用走完，全程没有一次绕行。
#[test]
fn the_revise_round_trip_over_mcp_is_three_calls() {
    let mut h = Harness::new();
    h.advance(StageId::Idea);
    h.advance(StageId::Selection);

    let mut even = fixtures::outputs(StageId::Script);
    for (i, beat) in even["script"]["story_arc"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        beat["start"] = json!(i as f64 * 2.0);
        beat["end"] = json!((i as f64 + 1.0) * 2.0);
        beat["duration_seconds"] = json!(2.0);
    }
    let (env, err) = h.call(
        "studio.submit_stage",
        json!({ "outputs": even, "summary": "每镜头 2 秒",
                "confirmation": fixtures::confirmation(StageId::Script) }),
    );
    assert!(!err);
    assert_eq!(env["waiting_on"], "user");

    let (env, err) = h.call(
        "studio.revise",
        json!({ "stage": "script", "message": "不要固定2秒，要根据镜头内容智能分配" }),
    );
    assert!(!err, "修订永远不该失败：{env}");
    assert_eq!(env["waiting_on"], "agent");
    assert!(env["pending_question"].is_null());

    let (env, err) = h.submit(StageId::Script);
    assert!(!err, "修订之后必须能立刻重新提交：{env}");
    assert_eq!(env["pending_question"]["question_id"], "script.approval");
}

#[test]
fn errors_come_back_as_an_envelope_with_a_remedy() {
    let mut h = Harness::new();
    let mut bad = fixtures::outputs(StageId::Idea);
    bad["brief"]["concepts"][0]
        .as_object_mut()
        .unwrap()
        .remove("story_beats");

    let (env, err) = h.call("studio.submit_stage", json!({ "outputs": bad }));
    assert!(err, "schema 不合规必须报错");
    let blocked = &env["blocked_by"];
    assert_eq!(blocked["code"], "schema_violation");
    assert!(blocked["message"]
        .as_str()
        .unwrap()
        .contains("brief.concepts[0].story_beats"));
    let remedy = blocked["remedy"].as_str().unwrap();
    assert!(!remedy.is_empty(), "blocked_by 必须带 remedy");
    assert!(
        remedy.contains("studio.schema"),
        "remedy 要指向能调的工具：{remedy}"
    );
}

#[test]
fn choosing_the_revise_option_over_mcp_sends_the_stage_back() {
    let mut h = Harness::new();
    h.advance(StageId::Idea);
    let (env, _) = h.submit(StageId::Selection);
    let qid = env["pending_question"]["question_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (env, err) = h.call(
        "studio.answer",
        json!({ "question_id": qid, "answer": "revise" }),
    );
    assert!(!err);
    assert_eq!(
        env["project"]["stage"], "selection",
        "选『先修改』不推进阶段"
    );
    assert_eq!(env["waiting_on"], "agent");
}

#[test]
fn retry_stage_tool_rejects_a_non_deterministic_stage_over_mcp() {
    let mut h = Harness::new();
    let (env, err) = h.call("studio.retry_stage", json!({ "stage": "idea" }));
    assert!(err, "idea 不是确定性阶段，retry_stage 应当拒绝");
    assert_eq!(env["blocked_by"]["code"], "invalid_transition");
    assert!(env["blocked_by"]["remedy"]
        .as_str()
        .unwrap()
        .contains("studio.revise"));
}

/// 留痕要能支撑生产环境的端到端报告：每次调用一条，出错时记下错误码
/// 以及那条阻塞有没有给出补救路径。
#[test]
fn every_call_is_traced_for_the_production_report() {
    let mut h = Harness::new();
    h.advance(StageId::Idea); // 无门：1 次 submit
    h.advance(StageId::Selection); // 有门：1 次 submit + 1 次 answer
    let (_, err) = h.call("studio.submit_stage", json!({ "outputs": { "nope": {} } }));
    assert!(err);

    let records = Trace::read(&h.root);
    assert_eq!(records.len(), 4, "调用次数应当精确可核账，实际 {records:?}");

    let by_tool = |name: &str| records.iter().filter(|r| r.tool == name).count();
    assert_eq!(by_tool("studio.submit_stage"), 3);
    assert_eq!(by_tool("studio.answer"), 1);

    let failed: Vec<_> = records.iter().filter(|r| !r.ok).collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].error_code.as_deref(), Some("schema_violation"));
    assert_eq!(
        failed[0].remedy_present,
        Some(true),
        "报告要能核对每条阻塞是否带补救路径"
    );

    // 成功的调用记下了当时该谁行动，报告据此还原阶段推进过程。
    assert!(records
        .iter()
        .any(|r| r.ok && r.waiting_on.as_deref() == Some("user")));
    assert!(records.iter().all(|r| r.duration_ms < 60_000));
}

/// 留痕记的是「作用在哪个阶段」，不是「调用之后停在哪个阶段」。
///
/// 这两者差一步：提交 idea 成功之后信封已经指向 selection。
/// 按后者归因的话 idea 永远不会出现在报告里——真实跑一遍就抓到了这个。
#[test]
fn trace_attributes_calls_to_the_stage_they_acted_on() {
    let mut h = Harness::new();
    h.advance(StageId::Idea);
    h.advance(StageId::Selection);

    let records = Trace::read(&h.root);
    let stages: Vec<&str> = records.iter().filter_map(|r| r.stage.as_deref()).collect();
    assert!(
        stages.contains(&"idea"),
        "提交 idea 应当记在 idea 名下，实际：{stages:?}"
    );
    assert!(stages.contains(&"selection"));
    assert!(
        !stages.contains(&"script"),
        "还没做剧本，不该有剧本的调用记录：{stages:?}"
    );

    // 确认门的应答记在门所属的阶段上
    let answer = records.iter().find(|r| r.tool == "studio.answer").unwrap();
    assert_eq!(answer.stage.as_deref(), Some("selection"));
}

#[test]
fn a_directory_that_is_not_a_project_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = Server::new(dir.path(), None);
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": { "name": "studio.status", "arguments": {} } });
    let raw = server.handle_line(&req.to_string()).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert!(v["result"]["isError"].as_bool().unwrap());
    let blocked = &v["result"]["structuredContent"]["blocked_by"];
    assert_eq!(blocked["code"], "not_a_project");
    // 这条 remedy 会原样进 Agent 的上下文，所以不提二进制名，
    // 而是把「新建作品」这个动作推给用户。见 docs/decisions/ADR-0002。
    let remedy = blocked["remedy"].as_str().unwrap();
    assert!(remedy.contains("用户"), "{remedy}");
    assert!(!remedy.contains("studiod"), "{remedy}");
}

/// 内容自评走的是同一条协议，不是内部捷径。
///
/// 这里不跑到 review（那要 GPU 和 ffmpeg），只验协议面：工具在册、
/// schema 把五个维度和时间点写清楚了、还没到验收就调会被结构化拒绝。
#[test]
fn self_review_is_on_the_tool_surface_with_the_rubric_spelled_out() {
    let mut h = Harness::new();
    let resp = h.rpc("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let t = tools
        .iter()
        .find(|t| t["name"] == "studio.self_review")
        .expect("内容自评必须在工具面上——不在工具面上的能力等于不存在");

    let items = &t["inputSchema"]["properties"]["items"];
    assert_eq!(items["minItems"], 5, "五个维度一个都不能少");
    let criteria = items["items"]["properties"]["criterion"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(criteria.len(), 5);
    assert!(criteria.iter().any(|c| c == "hook"));
    assert!(items["items"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == "at_seconds"));
}

#[test]
fn self_review_before_the_film_exists_is_refused_with_a_remedy() {
    let mut h = Harness::new();
    let (env, err) = h.call(
        "studio.self_review",
        json!({
            "items": [{ "criterion": "hook", "verdict": "met",
                        "at_seconds": 0.6, "evidence": "还没有片子可看，这条只是占位用的文字" }],
            "summary": "还没有片子"
        }),
    );
    assert!(err, "验收还没做，内容自评无从谈起");
    let blocked = &env["blocked_by"];
    assert_eq!(blocked["code"], "stage_not_ready");
    assert!(!blocked["remedy"].as_str().unwrap().is_empty());
}

/// 单入口之后 `studio.comfy.exclude_node` 的语义不存在了——排除唯一那个
/// 地址等于关掉渲染。它必须从工具面上彻底消失，而不是留一个骗人的 no-op。
#[test]
fn the_removed_exclude_node_tool_is_gone_from_the_surface() {
    let mut h = Harness::new();
    let resp = h.rpc("tools/list", json!({}));
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.iter().any(|n| n.contains("exclude_node")),
        "工具面上不该还有 exclude_node：{names:?}"
    );

    // 未知工具走 JSON-RPC 的 invalid params，不是决策信封里的 blocked_by。
    let resp = h.rpc(
        "tools/call",
        json!({ "name": "studio.comfy.exclude_node", "arguments": { "node": "http://x" } }),
    );
    assert_eq!(
        resp["error"]["code"], -32602,
        "调用一个已删除的工具必须报错，不能静默成功：{resp}"
    );
}
