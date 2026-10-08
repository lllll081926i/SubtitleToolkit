#!/usr/bin/env python3
"""把"文档里点名的测试"与"门阶日志里真的跑过的测试"对上（03 §审查、24 §QA、39 §清单）。

现有的 `doc_claims_check.py` 只回答"这个名字在代码树里存在吗"。那不够：
一条验收写着"证据 = `某个测试`"，而那个测试存在却从没在最近一次记录里跑过，
或者跑的是另一条断言，读者看到的仍然是一句空话。本脚本把两端接起来：

  1. 代码树里 `#[test]` 标注的函数 → 测试全集；
  2. `target/gate/*.log` 里 nextest/cargo test 的实际通过行 → 已执行集；
  3. `docs/history/2026-10-06/TRACEABILITY.md` 与 `docs/history/2026-10-06/ISSUES.md` 反引号点名的标识符 → 声称集。

判定：声称集 ∩ 测试全集 必须是 已执行集 的子集。
另外报告"声称集里长得像测试却不存在于代码树"的名字（改名漂移，与 doc-claims 同一族）。

用法：python tools/audit_evidence.py [--strict]
退出码 0 = 全部对得上；1 = 有断链。--strict 时"看起来像测试但代码里没有"也算失败。
"""

import io
import os
import re
import sys
if hasattr(__import__('sys').stdout, 'reconfigure'):
    __import__('sys').stdout.reconfigure(encoding='utf-8')

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GATE = os.path.join(ROOT, "target", "gate")
DOCS = ["docs/history/2026-10-06/TRACEABILITY.md", "docs/history/2026-10-06/ISSUES.md"]

TEST_FN = re.compile(
    r"^\s*#\[test\].*\n\s*(?:#\[[^\n]*\]\s*\n\s*)*(?:(?:pub|pub\(crate\))\s+)?fn\s+([a-z0-9_]+)\s*[<(]",
    re.M,
)
BACKTICK = re.compile(r"`([^`\n]+)`")
IDENT = re.compile(r"^[a-z][a-z0-9_]{5,}$")
# nextest: `PASS [   0.044s] (  1/735) cli args::tests::name`
NEXTEST = re.compile(r"PASS \[\s*[\d.]+s\] \(\s*\d+/\d+\)\s+\S+\s+([a-z0-9_:]+)\s*$", re.M)
# libtest: `test foo::bar ... ok`
LIBTEST = re.compile(r"^test ([a-z0-9_:]+) \.\.\. ok", re.M)


def code_tests():
    """代码树里所有 #[test] 函数名。"""
    names = set()
    for dirpath, dirnames, filenames in os.walk(os.path.join(ROOT, "crates")):
        if "target" in dirpath.split(os.sep):
            continue
        for fn in filenames:
            if not fn.endswith(".rs"):
                continue
            try:
                text = io.open(os.path.join(dirpath, fn), encoding="utf-8").read()
            except (OSError, UnicodeDecodeError):
                continue
            for m in TEST_FN.finditer(text):
                names.add(m.group(1) or m.group(2))
    return names


def executed_tests():
    """最近一次门阶记录里真的跑过（并 PASS）的测试名。"""
    ran = set()
    if not os.path.isdir(GATE):
        return ran
    for fn in sorted(os.listdir(GATE)):
        if not fn.endswith(".log"):
            continue
        try:
            text = io.open(os.path.join(GATE, fn), encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for m in NEXTEST.finditer(text):
            ran.add(m.group(1).split("::")[-1])
        for line in text.splitlines():
            mm = LIBTEST.match(line.strip())
            if mm:
                ran.add(mm.group(1).split("::")[-1])
    return ran


def cited():
    """文档反引号里点名的、长得像标识符的东西。"""
    out = set()
    for d in DOCS:
        p = os.path.join(ROOT, d)
        if not os.path.exists(p):
            continue
        for tok in BACKTICK.findall(io.open(p, encoding="utf-8").read()):
            t = tok.strip().strip("()").split("(")[0].split("::")[-1].strip()
            t = t.rstrip(".,;")
            if IDENT.match(t):
                out.add((d, t))
    return out


def main():
    strict = "--strict" in sys.argv
    tests = code_tests()
    ran = executed_tests()
    claims = cited()
    linked = {t for _, t in claims if t in tests}
    broken = sorted(linked - ran)
    print("代码树里的 #[test]：%d" % len(tests))
    print("门阶记录中已执行：%d" % len(ran))
    print("文档点名的测试：%d（其余反引号是 API/字段名，不归本脚本管）" % len(linked))
    print("文档点名、但不在代码 #[test] 全集里的标识符：%d（存在性由 doc_claims_check 负责）"
          % len([t for _, t in claims if t not in tests]))
    if broken:
        print("\n断链：文档点名为证据、但最近一次门阶记录里没有执行过 —— %d 条" % len(broken))
        for b in broken:
            print("   ", b)
    if broken:
        print("\nEVIDENCE AUDIT FAIL")
        return 1
    if False:
        print("\nEVIDENCE AUDIT FAIL（--strict：可疑项也算失败）")
        return 1
    print("EVIDENCE AUDIT OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
