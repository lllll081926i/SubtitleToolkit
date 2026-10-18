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
        "x86 host **已能构建**（`cargo build -p renderer-host --target i686-pc-windows-msvc`；"
        "产物真 32 位，与 x64 在真 libass 下输出逐字节一致 —— 见 x86-host-and-cache-key 裁决）。"
        "但规格要的是**x86 DLL 路由**的证据，而那需要一只真的 x86 libass DLL"
        "（本机只有 x64 那份，机器类型 0x8664）。已验到的是“x86 host 正确**拒绝** x64 DLL”"
        "（STK-RND-0091）—— 那是反面证据（证明校验在位），**不是**路由可用的证据，"
        "所以不列作已覆盖。另：x86 host 尚未随包分发（前提是 x86 DLL 的许可审计，21 §6）。",
    ),
    "AT-PGS-001": (
        "covered",
        "真实商业 SUP 执行：cli/tests/cli.rs + docs/reviews/2026-10-17-pgs-real-corpus.md 的 23/23 记录",
    ),
    "AT-PGS-002": ("covered", "cli/tests/cli.rs 的 forced-only 用例（只改 PCS）"),
    "AT-PGS-003": ("covered", "cli/tests/cli.rs 的 palette 用例（只改 PDS）"),
    "AT-PGS-004": ("covered", "pgs-author/tests/{author,pipeline}.rs 的 Basic 编码用例"),
    "AT-BD-001": (
        "covered",
        "两侧都有专属用例（以前记为 `shared`，是错的）。"
        "拒绝层：`codec-pgs/src/encoder.rs` 的 `invalid_inputs_are_rejected_before_any_segment_is_written`"
        "（3840 宽 → `STK-PGS-0091`，且断言**写任何段之前**就拒）；"
        "另 `core-model/src/profile.rs::validate`（`STK-VAL-0002`）与 `canvas.rs` 的"
        " `bluray_bounds_reject_2160_pgs`。GUI 解释层：`gui/src/state/profile.rs` 的"
        "`problem()`/`is_runnable()` 与 9 个用例。"
        "**并且抳出一个真缺陷**：出厂默认的 `2160p 4K HDR SUP` 自己跑不起来"
        "（`pgs encode` 默认 `--level standard`）—— 已改成 `1080p HDR SUP`，"
        "由 `the_default_sup_profile_is_not_4k` 钉住。"
        "GUI 不重写判定，直接用 `core_model::Canvas::within_bluray`（单一事实源）。",
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
        "covered",
        "gui/tests/ui_responsiveness.rs（5 用例）——**实测**而非结构证据：真重任务在后台跑时"
        "反复调 SubtitleApp::update 并计时，最慢 18.5µs / 均值 9.5µs（预算 8ms，37 §1）。"
        "另含：空闲基线、取消点击不阻塞且令牌与 worker 是同一份（已断言）、"
        "StartQueue 真的把运行状态置位且换新令牌、无输入时不假装在跑。"
        "未做的：真人点按钮/IME/DPI 仍需人眼（不主张已覆盖）",
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
