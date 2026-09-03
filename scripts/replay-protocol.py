#!/usr/bin/env python3
"""协议层冒烟：不用 Codex，直接跟 studiod serve 说 JSON-RPC，走完提交给
ComfyUI 之前的六个阶段，中间重放一次「不要固定 2 秒」的修订。

这**不是**端到端测试——真正的端到端需要一个真实 Codex 会话，只在生产环境跑，
见 docs/e2e.md。这个脚本用来在换了机器、换了构建之后确认协议层还是通的。

    cargo build --release -p studiod
    cargo run -q -p studio-core --features fixtures --example export_fixtures > /tmp/fixtures.json
    ./target/release/studiod init /tmp/replay.studio
    python3 scripts/replay-protocol.py /tmp/replay.studio /tmp/fixtures.json
    cd /tmp/replay.studio && studiod e2e report
"""

import json
import subprocess
import sys
from pathlib import Path

STAGES = ["idea", "selection", "script", "storyboard", "visual_assets", "prompt_pack"]


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    bundle, fixtures_path = Path(sys.argv[1]), Path(sys.argv[2])
    binary = Path(__file__).resolve().parent.parent / "target/release/studiod"
    fixtures = json.loads(fixtures_path.read_text())

    proc = subprocess.Popen(
        [str(binary), "serve"], cwd=bundle,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
    )
    counter = [0]

    def rpc(method, params=None):
        counter[0] += 1
        msg = {"jsonrpc": "2.0", "id": counter[0], "method": method, "params": params or {}}
        proc.stdin.write(json.dumps(msg) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    def call(name, args):
        result = rpc("tools/call", {"name": name, "arguments": args})["result"]
        return result["structuredContent"], result.get("isError", False)

    rpc("initialize", {"protocolVersion": "2025-06-18"})

    for stage in STAGES:
        fix = fixtures[stage]
        args = {"outputs": fix["outputs"], "summary": fix["summary"]}
        if fix.get("confirmation"):
            args["confirmation"] = fix["confirmation"]

        # 重放 2026-09-03 那次：先交一版每镜头 2 秒的，再由用户要求智能分配。
        if stage == "script":
            even = json.loads(json.dumps(args["outputs"]))
            for i, beat in enumerate(even["script"]["story_arc"]):
                beat["start"], beat["end"], beat["duration_seconds"] = i * 2.0, (i + 1) * 2.0, 2.0
            env, err = call("studio.submit_stage", {**args, "outputs": even, "summary": "每镜头 2 秒"})
            assert not err, env
            print(f"  script  交了平均时长版 -> 等 {env['waiting_on']}")
            env, err = call("studio.revise",
                            {"stage": "script", "message": "不要固定2秒，要根据镜头内容智能分配"})
            assert not err, env
            print(f"  script  用户要求智能分配 -> 等 {env['waiting_on']}")

        env, err = call("studio.submit_stage", args)
        if err:
            print(f"提交 {stage} 失败：{json.dumps(env, ensure_ascii=False, indent=2)}")
            return 1
        question = env.get("pending_question")
        if question:
            env, err = call("studio.answer",
                            {"question_id": question["question_id"], "answer": "approve"})
            if err:
                print(f"确认 {stage} 失败：{json.dumps(env, ensure_ascii=False, indent=2)}")
                return 1
        print(f"  {stage:<14} 完成 -> {env['project']['stage']} ({env['progress']['completed']}/9)")

    env, _ = call("studio.status", {})
    proc.stdin.close()
    proc.wait()

    ok = env["project"]["stage"] == "render" and env["progress"]["completed"] == 6
    print(f"\n{'通过' if ok else '未通过'}：停在 {env['project']['stage']}，"
          f"等 {env['waiting_on']}，完成 {env['progress']['completed']}/9")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
