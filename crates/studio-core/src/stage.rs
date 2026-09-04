//! 阶段图。这是流程骨架的唯一事实源——`emit-assets` 从这里生成文档里的阶段表。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 十个阶段，顺序固定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageId {
    Idea,
    Selection,
    Script,
    Storyboard,
    VisualAssets,
    PromptPack,
    Preview,
    Render,
    Post,
    Review,
}

/// 承担该阶段的能力（对应一个 Skill 目录名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Idea,
    Selection,
    Script,
    Director,
    Visual,
    Prompt,
    Comfyui,
    Post,
    Review,
}

/// 阶段类型，决定谁产出内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    /// 产物全部由 Agent 给出，控制面不注册执行器。
    Creative,
    /// Agent 定内容，确认后由控制面执行生成。
    Hybrid,
    /// 纯代码，失败时才回到 Agent。
    Deterministic,
}

#[derive(Debug, Clone, Copy)]
pub struct StageSpec {
    pub id: StageId,
    pub capability: Capability,
    pub kind: StageKind,
    /// 确认门 id。门在阶段**产出之后**暂停。
    pub gate: Option<&'static str>,
    /// 产物在 outputs 里的顶层键。
    pub output_key: &'static str,
}

/// 阶段图。顺序即执行顺序。
pub const STAGE_GRAPH: [StageSpec; 10] = [
    StageSpec {
        id: StageId::Idea,
        capability: Capability::Idea,
        kind: StageKind::Creative,
        gate: None,
        output_key: "brief",
    },
    StageSpec {
        id: StageId::Selection,
        capability: Capability::Selection,
        kind: StageKind::Creative,
        gate: Some("selection.approval"),
        output_key: "selection",
    },
    StageSpec {
        id: StageId::Script,
        capability: Capability::Script,
        kind: StageKind::Creative,
        gate: Some("script.approval"),
        output_key: "script",
    },
    StageSpec {
        id: StageId::Storyboard,
        capability: Capability::Director,
        kind: StageKind::Creative,
        gate: Some("storyboard.approval"),
        output_key: "storyboard",
    },
    StageSpec {
        id: StageId::VisualAssets,
        capability: Capability::Visual,
        kind: StageKind::Hybrid,
        gate: Some("visual_assets.approval"),
        output_key: "asset_plan",
    },
    StageSpec {
        id: StageId::PromptPack,
        capability: Capability::Prompt,
        kind: StageKind::Creative,
        gate: Some("prompt_pack.approval"),
        output_key: "prompt_pack",
    },
    StageSpec {
        id: StageId::Preview,
        capability: Capability::Comfyui,
        kind: StageKind::Deterministic,
        // 花 GPU 时间的两级阶梯：先出便宜的 480p，人工确认构图/内容没问题，
        // 再花贵的时间出正式尺寸。这是确定性阶段里唯一带确认门的一个——
        // render 本身没法在执行器内部插一段确认，只能在它前面插一整个阶段。
        gate: Some("preview.approval"),
        output_key: "preview",
    },
    StageSpec {
        id: StageId::Render,
        capability: Capability::Comfyui,
        kind: StageKind::Deterministic,
        gate: None,
        output_key: "render",
    },
    StageSpec {
        id: StageId::Post,
        capability: Capability::Post,
        kind: StageKind::Deterministic,
        gate: None,
        output_key: "post",
    },
    StageSpec {
        id: StageId::Review,
        capability: Capability::Review,
        kind: StageKind::Deterministic,
        gate: None,
        output_key: "review",
    },
];

impl StageId {
    pub fn spec(self) -> &'static StageSpec {
        // STAGE_GRAPH 的顺序与 StageId 的声明顺序一致，index 即序号。
        &STAGE_GRAPH[self.index()]
    }

    pub fn index(self) -> usize {
        match self {
            StageId::Idea => 0,
            StageId::Selection => 1,
            StageId::Script => 2,
            StageId::Storyboard => 3,
            StageId::VisualAssets => 4,
            StageId::PromptPack => 5,
            StageId::Preview => 6,
            StageId::Render => 7,
            StageId::Post => 8,
            StageId::Review => 9,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StageId::Idea => "idea",
            StageId::Selection => "selection",
            StageId::Script => "script",
            StageId::Storyboard => "storyboard",
            StageId::VisualAssets => "visual_assets",
            StageId::PromptPack => "prompt_pack",
            StageId::Preview => "preview",
            StageId::Render => "render",
            StageId::Post => "post",
            StageId::Review => "review",
        }
    }

    /// 修订时应当退回到哪个阶段。默认是自身；`preview` 是例外——它自己不产出
    /// 独立内容，问题一定出在 `prompt_pack` 决定的内容上，没有更细粒度的
    /// 回退点，所以修订 `preview`（不论是直接调 `studio.revise("preview", ..)`
    /// 还是在它的确认门上选了「有问题」）一律退回 `prompt_pack`。
    pub fn revise_target(self) -> StageId {
        match self {
            StageId::Preview => StageId::PromptPack,
            other => other,
        }
    }

    pub fn parse(s: &str) -> Option<StageId> {
        STAGE_GRAPH
            .iter()
            .find(|sp| sp.id.as_str() == s)
            .map(|sp| sp.id)
    }

    /// 下一个阶段；最后一个阶段返回 None。
    pub fn next(self) -> Option<StageId> {
        STAGE_GRAPH.get(self.index() + 1).map(|s| s.id)
    }

    pub fn first() -> StageId {
        STAGE_GRAPH[0].id
    }

    pub fn all() -> impl Iterator<Item = StageId> {
        STAGE_GRAPH.iter().map(|s| s.id)
    }

    pub fn gate(self) -> Option<&'static str> {
        self.spec().gate
    }

    pub fn capability(self) -> Capability {
        self.spec().capability
    }

    pub fn kind(self) -> StageKind {
        self.spec().kind
    }

    /// 产物在 outputs 里的顶层键。
    pub fn output_key(self) -> &'static str {
        self.spec().output_key
    }
}

impl fmt::Display for StageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Idea => "idea",
            Capability::Selection => "selection",
            Capability::Script => "script",
            Capability::Director => "director",
            Capability::Visual => "visual",
            Capability::Prompt => "prompt",
            Capability::Comfyui => "comfyui",
            Capability::Post => "post",
            Capability::Review => "review",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl StageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StageKind::Creative => "creative",
            StageKind::Hybrid => "hybrid",
            StageKind::Deterministic => "deterministic",
        }
    }
}

/// 全部 Skill 目录名。`run-management` 不对应阶段，单列。
pub const SKILL_NAMES: [&str; 10] = [
    "idea",
    "selection",
    "script",
    "director",
    "visual",
    "prompt",
    "comfyui",
    "post",
    "review",
    "run-management",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_graph_order() {
        for (i, spec) in STAGE_GRAPH.iter().enumerate() {
            assert_eq!(
                spec.id.index(),
                i,
                "{} 的 index 与 STAGE_GRAPH 顺序不一致",
                spec.id
            );
            assert_eq!(spec.id.spec().id, spec.id);
        }
    }

    #[test]
    fn parse_roundtrip() {
        for stage in StageId::all() {
            assert_eq!(StageId::parse(stage.as_str()), Some(stage));
        }
        assert_eq!(StageId::parse("nope"), None);
    }

    #[test]
    fn six_gates_on_the_expected_stages() {
        let gated: Vec<_> = StageId::all().filter(|s| s.gate().is_some()).collect();
        assert_eq!(
            gated,
            vec![
                StageId::Selection,
                StageId::Script,
                StageId::Storyboard,
                StageId::VisualAssets,
                StageId::PromptPack,
                StageId::Preview,
            ],
            "确认门位置变了——这是产品决策，改动需要同步 docs/state-machine.md"
        );
    }

    /// preview 是唯一「控制面自动执行、但仍带确认门」的阶段——修订它必须
    /// 落到 prompt_pack，因为它自己不产出独立内容，没有更细的回退点。
    #[test]
    fn preview_revises_back_to_prompt_pack() {
        assert_eq!(StageId::Preview.revise_target(), StageId::PromptPack);
        for stage in StageId::all() {
            if stage != StageId::Preview {
                assert_eq!(stage.revise_target(), stage, "{stage} 不该被重定向");
            }
        }
    }

    #[test]
    fn idea_and_review_have_no_gate() {
        assert!(StageId::Idea.gate().is_none());
        assert!(StageId::Review.gate().is_none());
    }

    #[test]
    fn chain_is_linear_and_terminates() {
        let mut s = StageId::first();
        let mut n = 1;
        while let Some(next) = s.next() {
            s = next;
            n += 1;
        }
        assert_eq!(n, 10);
        assert_eq!(s, StageId::Review);
    }
}
