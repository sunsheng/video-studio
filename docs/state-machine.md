# 状态机

## typestate

阶段状态编码进类型参数，转换消耗自身：

```rust
impl Stage<Draft> {
    fn submit(self, outputs: Outputs, gate: Option<GateId>) -> Result<Stage<AwaitingConfirmation>>;
}
impl Stage<AwaitingConfirmation> {
    fn approve(self, answer: Answer) -> Result<Stage<Approved>>;
    fn revise(self, message: &str) -> Stage<Draft>;   // 消耗自身，占用与门必然一同释放
}
```

`Stage<AwaitingConfirmation>` 上**没有** `submit` 方法。

## 验收标准

把旧实现那个 bug 写成 Rust——revise 之后不释放就再 submit——**必须编译不过**。
`tests/typestate_compile_fail.rs` 用 `trybuild` 之外的方式守这条：
见 `crates/studio-core/src/state.rs` 顶部的 `compile_fail` doctest。

## 阶段图

| # | stage | capability | kind | 确认门 |
|---|---|---|---|---|
| 1 | idea | idea | creative | — |
| 2 | selection | selection | creative | `selection.approval` |
| 3 | script | script | creative | `script.approval` |
| 4 | storyboard | director | creative | `storyboard.approval` |
| 5 | visual_assets | visual | hybrid | `visual_assets.approval` |
| 6 | prompt_pack | prompt | creative | `prompt_pack.approval` |
| 7 | preview | comfyui | deterministic | `preview.approval` |
| 8 | render | comfyui | deterministic | — |
| 9 | post | post | deterministic | — |
| 10 | review | review | deterministic | — |

门在阶段**产出之后**暂停。`prompt_pack` 上那道门是花 GPU 时间前的最后一关。

`preview` 是唯一一个「控制面自动执行、但仍带确认门」的阶段：执行完不直接判
`Approved`，而是走跟 Agent 提交带门阶段完全一样的 `AwaitingConfirmation`
状态挂起，等 480p 预览的构图/内容被确认，才轮到花钱的正式 `render`。
它自己不产出独立内容，所以它的门选「有问题」（或直接对它调
`studio.revise`）统一重定向退回 `prompt_pack`，不是退回 `preview` 自己——
`StageId::revise_target()` 是这条重定向规则的唯一事实源。

## 并发

一个 bundle 一个进程，`.studio/studiod.lock` 用 advisory flock。
第二个会话打开同一 bundle 立即返回 `project_busy` 并附持有者 PID，不排队。
跨 bundle 完全独立。
