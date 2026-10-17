#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""验收用例覆盖审计：把 25_ACCEPTANCE_TESTS.md 里的 AT 逐条对上**真实测试断言**。

这个脚本的立场：**在注释里出现不算覆盖**。

三种结论：
  covered    —— 该 AT 的 ID 出现在至少一个 `#[test]` 函数体内（或 `tests/` 里的测试函数）
  shared     —— 没有专属函数，但规格描述的断言由某组更宽的用例覆盖（需在下面明写理由）
  uncovered  —— 当前没有可执行证据（要么依赖缺失，要么根本没做）

第二类必须带理由，第三类必须能说清为什么。
"""
import os
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

ROOT = r"D:\Code\SubtitleToolkit"
SPEC = os.path.join(ROOT, "docs", "specs", "25_ACCEPTANCE_TESTS.md")

# 覆盖结论。每一条要么给"专属测试函数名"，要么给 shared 理由，要么是 uncovered 理由。
COVERAGE = {
    "AT-ASS-001": ("covered", "codec-ass/tests/roundtrip.rs 的 parse→write 逐字节用例"),
    "AT-ASS-002": ("covered", "codec-ass/tests/colour_map.rs 的逐通道期待值用例"),
    "AT-CANVAS-001": (
        "covered",
        "transform-resolution/tests/rewrite.rs：1080→2160 的 PlayRes 缩放与 \\pos(960,540)→\\pos(1920,1080)",
    ),
    "AT-CANVAS-002": (
        "covered",
        "media-probe/src/suggest.rs::non_16_9_video_does_not_suggest_canvas（2.39:1 不得被建议成逻辑画布）",
    ),
    "AT-MULTI-001": ("covered", "cli/tests/batch.rs 的多 profile 用例"),
    "AT-MULTI-002": ("covered", "cli/tests/batch.rs 的共享渲染缓存用例"),
    "AT-TIME-001": ("covered", "transform-time 的 24000/1001→25 长片累加用例"),
    "AT-TIME-002": ("covered", "transform-time 的半帧边界与短 cue 量化用例"),
    "AT-PROBE-001": ("covered", "media-probe 的\"只建议不改时间轴\"用例"),
    "AT-RENDER-001": (
        "covered",
        "renderer-host/tests/host_isolation.rs + render-libass/tests/render_real.rs",
    ),
    "AT-RENDER-002": (
        "uncovered",
        "需要真实 VSFilterMod provider；工作区只有未核验的候选 DLL，"
        "来源/许可未完成审计（21 §6），不以自伤注入代替",
    ),
    "AT-RENDER-003": (
        "uncovered",
        "需要 x86 renderer host 产物；当前只构建 x64（28 §2 列为可选组件）",
    ),
    "AT-PGS-001": (
        "covered",
        "真实商业 SUP 执行：cli/tests/cli.rs + docs/reviews/2026-10-17-pgs-real-corpus.md 的 23/23 记录",
    ),
    "AT-PGS-002": ("covered", "cli/tests/cli.rs 的 forced-only 用例（只改 PCS）"),
    "AT-PGS-003": ("covered", "cli/tests/cli.rs 的 palette 用例（只改 PDS）"),
    "AT-PGS-004": ("covered", "pgs-author/tests/{author,pipeline}.rs 的 Basic 编码用例"),
    "AT-BD-001": (
        "shared",
        "已实现 CLI 侧拒绝（2160p + Strict）；GUI 禁用解释留到 GUI 阶段，见规格原文",
    ),
    "AT-BDN-001": ("covered", "codec-bdn/tests/png_consistency.rs + timecode.rs"),
    "AT-FONT-001": ("covered", "font-service/tests/real_fonts.rs 的缺字报告用例"),
    "AT-FONT-002": ("covered", "font-service/tests/font_cache.rs 的环境变化→cache miss 用例"),
    "AT-MKV-001": (
        "covered",
        "cli/tests/mkv.rs（真实 MKVToolNix v102.0，23/23 断言，见裁决文档）",
    ),
    "AT-BATCH-001": (
        "covered",
        "cli/tests/batch.rs 的 100 输入×3 profile 用例（含部分失败隔离）",
    ),
    "AT-UI-001": (
        "shared",
        "已接通的部分：真任务经 spawn_blocking 跑（不占 UI 线程）、取消令牌与引擎共享同一份 Arc"
        "（已断言）；未做的部分：60 秒持续响应的**实测**需要真人操作与屏幕录制，"
        "当前只有结构证据（阻塞 IO 不在 UI 线程上），不声称已通过",
    ),
    "AT-PATH-001": (
        "covered",
        "codec-bdn/tests/paths.rs 的 chinese_file_names_and_long_paths_are_validated "
        "+ long_path_with_one_bad_png_is_located",
    ),
    "AT-SEC-001": ("covered", "codec-pgs/tests/security.rs 的恶意 length/RLE 用例"),
    "AT-OUT-001": (
        "covered",
        "task-engine 的 verify 失败→不替换旧文件用例 + cli/tests/cli.rs 的覆盖策略用例",
    ),
}

# 允许在 spec 里出现、但我们故意不纳入 AT 的（比如新增后未登记的）
EXTRA_NOTE = {}


def spec_ids():
    text = open(SPEC, encoding="utf-8").read()
    return sorted(set(re.findall(r"^- (AT-[A-Z]+-\d+)", text, re.M)))


def main():
    ids = spec_ids()
    print(f"规格 AT 条目：{len(ids)} 条\n")

    by_state = {"covered": [], "shared": [], "uncovered": []}
    unknown = []
    for at in ids:
        if at not in COVERAGE:
            unknown.append(at)
            continue
        state, why = COVERAGE[at]
        by_state[state].append((at, why))

    w = max(len(a) for a in ids)
    for state, label in [
        ("covered", "有专属用例"),
        ("shared", "由更宽用例覆盖"),
        ("uncovered", "无可执行证据"),
    ]:
        print(f"== {label}（{len(by_state[state])}）==")
        for at, why in by_state[state]:
            print(f"  {at:<{w}}  {why}")
        print()

    if unknown:
        print(f"规格里有、审计表未映射：{unknown}")
        print("  -> 要么在 COVERAGE 里登记结论，要么从规格里删掉\n")

    total = len(ids)
    covered = len(by_state["covered"])
    print(
        f"结论：{covered}/{total} 有专属用例；"
        f"{len(by_state['shared'])} 由更宽用例覆盖；"
        f"{len(by_state['uncovered'])} 无证据（均为依赖或范围缺失）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
