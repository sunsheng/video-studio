//! 脚本场景要能直接进 `cargo test --workspace`——这就是它们跟 Agent 场景
//! 的关键区别（见 ADR-0004）。这里的每个测试都跑一遍真实 studiod 二进制。

use studio_skill_eval::{all_scenarios, run_scenario};

fn assert_passed(id: &str) {
    let r = run_scenario(id).unwrap_or_else(|| panic!("场景 {id} 不存在"));
    assert!(
        r.passed,
        "场景 {id} 未通过：{:#?}",
        r.verdicts.iter().filter(|v| !v.passed).collect::<Vec<_>>()
    );
}

#[test]
fn golden_six_stage_with_revise_passes() {
    assert_passed("golden_six_stage_with_revise");
}

#[test]
fn concurrent_open_reports_busy_with_pid_passes() {
    assert_passed("concurrent_open_reports_busy_with_pid");
}

#[test]
fn the_registry_lists_every_scripted_scenario() {
    let ids: Vec<&str> = all_scenarios().into_iter().map(|m| m.id).collect();
    assert!(ids.contains(&"golden_six_stage_with_revise"));
    assert!(ids.contains(&"concurrent_open_reports_busy_with_pid"));
}

#[test]
fn an_unknown_scenario_id_returns_none_not_a_panic() {
    assert!(run_scenario("does-not-exist").is_none());
}
