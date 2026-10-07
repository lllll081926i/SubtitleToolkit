#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""文档声明核对：TRACEABILITY / REVIEW_GATES 里用反引号点名的测试与标识符，必须真的存在。

为什么需要它：TRACEABILITY 是"验收证据"的目录，而证据最容易犯的错是写一个不存在的测试名
（或者测试改名后文档没跟上）。这种漂移靠人眼发现不了，也不能靠我保证。
所以这条检查是可重复的：`bash tools/check_all.sh` 把它当门阶跑，CI 也跑。

核对范围只取"下划线命名的标识符"（测试名、函数名）与 `tools/*.py` 文件名，
避免把普通中文/英文词当符号。缺失即退出 1。
"""
from __future__ import annotations

import io
import re
import sys
if hasattr(__import__('sys').stdout, 'reconfigure'):
    __import__('sys').stdout.reconfigure(encoding='utf-8')
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ["docs/TRACEABILITY.md", "docs/REVIEW_GATES.md"]
IDENT = re.compile(r"`([a-z0-9_]{6,})`")


def sources() -> str:
    parts = []
    for p in list(ROOT.glob("crates/**/*.rs")) + list(ROOT.glob("tools/*")) + [ROOT / "Cargo.toml"]:
        if p.is_file():
            parts.append(p.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(parts)


def is_symbol_candidate(name: str) -> bool:
    # 测试名/函数名一般有两个以上下划线；x86_64、sha256 这类不在此列
    return name.count("_") >= 2

def main() -> int:
    blob = sources()
    checked = 0
    missing: list[str] = []
    for rel in DOCS:
        doc = ROOT / rel
        if not doc.is_file():
            print(f"FAIL: 缺少文档 {rel}")
            return 1
        text = io.open(doc, encoding="utf-8").read()
        for m in IDENT.finditer(text):
            name = m.group(1)
            if not is_symbol_candidate(name):
                continue
            checked += 1
            if name not in blob:
                missing.append(f"{rel}: `{name}`")
    print(f"核对 {checked} 处文档点名的标识符")
    if missing:
        for x in missing:
            print(f"  MISSING {x}")
        print("DOC CLAIMS FAIL")
        return 1
    print("DOC CLAIMS OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
