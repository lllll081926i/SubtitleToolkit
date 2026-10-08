#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成 THIRD_PARTY_NOTICES.md 并按 21 §1/§11 校验许可证白名单。

21 §11 要求：每个依赖记录 version、URL、license、hash、bundled/external、更新策略。
这份脚本从 `Cargo.lock` + 本机注册表缓存里取真实数据（不是手抄），并做两件事：

1. 写 `THIRD_PARTY_NOTICES.md`；
2. 任何不在 `deny.toml` 的 `[licenses].allow` 里的 SPDX 表达式都让脚本退出 1
   —— 也就是说这条门阶会在有人偷偷引入 GPL 依赖时变红，而不是等发布前才发现。

用法：python tools/notices.py [--check]   # --check 只校验不写文件
"""
from __future__ import annotations

import hashlib
import os
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# 门阶把这条的输出重定向进日志文件；Windows 控制台默认 cp936，会把中文结论写成非 UTF-8 字节，
# 于是 `target/gate/notices.log` 既不好读也没法 diff（与 tools/doc_claims_check.py 同一个处理）。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
CARGO_HOME = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))

# 21 §1 之外的组件：不是 crates.io 依赖，但会随包/随验证出现，必须单独交代
EXTRA = [
    ("libass", "ISC", "https://github.com/libass/libass",
     "external（native provider；本机已装，哈希逐条实测列在下方的锁表里）"),
    ("VSFilterMod.dll", "GPL-3.0（按上游项目推定；本 DLL 的源码映射未核实）", "https://github.com/sorayuki/VSFilterMod",
     "third_party/VSFilterMod.dll（项目内候选文件；未接入产品或安装包）"),
    ("Pillow", "MIT-CMU", "https://python-pillow.org",
     "dev-only（PNG 第三方交叉验证，不随安装包分发）"),
    ("pysubs2", "MIT", "https://github.com/timo0/pysubs2",
     "dev-only（ASS/SRT 语义交叉验证，不随安装包分发）"),
    ("cargo-deny", "MIT OR Apache-2.0", "https://github.com/EmbarkStudios/cargo-deny",
     "dev-only（许可与 RustSec 通告门阶）"),
    ("cargo-nextest", "Apache-2.0 OR MIT", "https://github.com/nextest-rs/nextest",
     "dev-only（测试门阶执行器）"),
    ("vcpkg", "MIT", "https://github.com/microsoft/vcpkg",
     "dev-only（构建 native 依赖的开发期工具，不随安装包分发）"),
]


def file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def resolve_outside_repo(spec: str) -> tuple[Path | None, str]:
    """锁文件里的产物路径是给人读的，可能带括号说明，也可能是相对仓库或相对仓库上级写的。

    返回 (找到的绝对路径或 None, 原样保留的说明)。找不到不算错误：这台机器上没装
    这个工具是真的，但清单里那条哈希仍然是锁定的事实 —— 区别必须写清楚，
    不能把"没实测"混成"实测通过"，那是 W3 原本的毛病。
    """
    head = re.split(r"[\s（(]", spec.strip(), 1)[0]
    for cand in (ROOT / head, ROOT.parent / head):
        if cand.is_file():
            return (cand, "")
    return (None, "本机没有这个文件（哈希按锁文件记录，未在这台机器上实测）")


def native_rows() -> tuple[list[dict], list[str]]:
    """从 third_party/SOURCE_AUDIT_LOCK.json 取 native/二进制产物，逐条实测哈希。"""
    lock = ROOT / "third_party" / "SOURCE_AUDIT_LOCK.json"
    if not lock.is_file():
        return ([], ["SOURCE_AUDIT_LOCK.json 不在仓库里：native 产物无人锁哈希"])
    import json

    data = json.loads(lock.read_text(encoding="utf-8"))
    rows: list[dict] = []
    problems: list[str] = []

    def add(component: str, version: str, lic: str, spec: str, expected: str) -> None:
        if not expected:
            problems.append("%s：%s 没有锁 SHA-256" % (component, spec))
            return
        found, note = resolve_outside_repo(spec)
        actual = ""
        status = "absent-here"
        if found is not None:
            actual = file_sha256(found)
            if actual == expected:
                status = "match"
            else:
                status = "MISMATCH"
                problems.append(
                    "%s：%s 实测与锁定的 SHA-256 不符（锁定 %s ｜ 实测 %s）"
                    % (component, spec, expected, actual)
                )
        rows.append(
            {
                "component": component,
                "version": version,
                "license": lic,
                "path": spec,
                "locked": expected,
                "actual": actual,
                "status": status,
                "note": note,
            }
        )

    for src in data.get("sources", []):
        nb = src.get("native_build") or {}
        for art in nb.get("artifacts", []):
            add(
                src.get("name", "?"),
                str(nb.get("version", "")),
                src.get("license", "?"),
                art.get("path", ""),
                art.get("sha256", ""),
            )
    for art in data.get("binary_artifacts", []):
        add(
            art.get("name", "?"),
            str(art.get("version_resource", "")),
            str(art.get("license", art.get("license_status", "来源/许可证待核验"))),
            art.get("path", ""),
            art.get("sha256", ""),
        )
    tools = data.get("verification_tools", {}) or {}
    for key, label in (("ffprobe_external", "ffprobe"), ("ffmpeg_external", "ffmpeg")):
        ent = tools.get(key)
        if isinstance(ent, dict):
            add(label, str(ent.get("version", "")), str(ent.get("license", "")), ent.get("path", ""), ent.get("sha256", ""))
    return rows, problems


def source_archive_rows() -> tuple[list[dict], list[str]]:
    """校验来源锁中随仓库保存的源码归档，并为声明清单生成记录。"""
    lock = ROOT / "third_party" / "SOURCE_AUDIT_LOCK.json"
    if not lock.is_file():
        return ([], ["SOURCE_AUDIT_LOCK.json 不在仓库里：源码归档无人锁哈希"])
    import json

    data = json.loads(lock.read_text(encoding="utf-8"))
    rows: list[dict] = []
    problems: list[str] = []
    for src in data.get("sources", []):
        artifact = src.get("artifact")
        expected = src.get("artifact_sha256")
        if not artifact:
            continue
        if not expected:
            problems.append("%s：源码归档 %s 没有锁 SHA-256" % (src.get("name", "?"), artifact))
            continue
        path = ROOT / artifact
        actual = file_sha256(path) if path.is_file() else ""
        status = "match" if actual == expected else ("absent" if not actual else "MISMATCH")
        if status != "match":
            problems.append(
                "%s：源码归档 %s 状态为 %s（锁定 %s ｜ 实测 %s）"
                % (src.get("name", "?"), artifact, status, expected, actual or "缺失")
            )
        version = src.get("release_version_file") or src.get("version") or ""
        if not version and src.get("commit_sha"):
            version = str(src["commit_sha"])
        rows.append(
            {
                "name": src.get("name", "?"),
                "version": version,
                "license": src.get("license", "UNKNOWN"),
                "license_notes": src.get("license_notes", ""),
                "source": src.get("repository", src.get("source", "")),
                "artifact": artifact,
                "locked": expected,
                "actual": actual,
                "status": status,
            }
        )
    return rows, problems


def parse_lock(path: Path) -> list[dict]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return [p for p in data.get("package", [])]


def crate_license(name: str, version: str) -> tuple[str | None, str | None]:
    """从解压出来的 crate 源码里读 license / repository（这才是发布出去的那份文本）。"""
    manifests = sorted((CARGO_HOME / "registry" / "src").glob(f"*/{name}-{version}/Cargo.toml"))
    if not manifests:
        return (None, None)
    manifest = manifests[0]
    text = manifest.read_text(encoding="utf-8", errors="replace")
    m = re.search(r'(?m)^license\s*=\s*"([^"]+)"', text)
    repo = re.search(r'(?m)^repository\s*=\s*"([^"]+)"', text)
    lic = m.group(1) if m else None
    if lic is None:
        expr = re.search(r'(?m)^license-file\s*=\s*"([^"]+)"', text)
        if expr:
            lic = "NONSTANDARD(license-file: " + expr.group(1) + ")"
    return (lic, repo.group(1) if repo else None)


def crate_sha256(name: str, version: str) -> str | None:
    archives = sorted((CARGO_HOME / "registry" / "cache").glob(f"*/{name}-{version}.crate"))
    if not archives:
        return None
    f = archives[0]
    h = hashlib.sha256()
    with f.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def allowed_licenses() -> set[str]:
    deny = tomllib.loads((ROOT / "deny.toml").read_text(encoding="utf-8"))
    return set(deny.get("licenses", {}).get("allow", []))


def split_expr(expr: str) -> list[str]:
    """把 SPDX 表达式拆成单个许可标识。

    现实里三种写法都出现过：`MIT OR Apache-2.0`、`MIT/Apache-2.0`（老式斜杠=OR）、
    `(MIT OR Apache-2.0) AND Unicode-3.0`。括号与空白都是噪声，剥掉再比。
    AND 的两侧都必须各自在白名单里，这才是"必须同时满足"的正确读法。
    """
    parts = re.split(r"(?i)\s+(?:OR|AND)\s+|\s*\|\|\s*|\s*/\s*", expr.strip())
    out = []
    for p in parts:
        token = p.strip(" ()[]{},").strip()
        if token:
            out.append(token)
    return out


def main(argv: list[str]) -> int:
    check = "--check" in argv
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        print("FAIL: 没有 Cargo.lock（复现能力要求它入库）")
        return 1
    allow = allowed_licenses()
    if not allow:
        print("FAIL: deny.toml 的 [licenses].allow 是空的")
        return 1
    rows = []
    problems = []
    for pkg in sorted(parse_lock(lock), key=lambda p: (p["name"], p.get("version", ""))):
        name, version = pkg["name"], pkg.get("version", "")
        source = pkg.get("source", "")
        if "registry+" not in source:
            continue  # 本地 workspace 成员，不属于第三方
        lic, repo = crate_license(name, version)
        sha = crate_sha256(name, version)
        if lic is None:
            problems.append("%s %s: 读不到 license 字段" % (name, version))
            lic = "UNKNOWN"
        else:
            for token in split_expr(lic):
                if token not in allow:
                    problems.append("%s %s: 许可 %r 不在 21 §1 白名单内" % (name, version, token))
        rows.append((name, version, lic, repo or "", sha or "", (pkg.get("checksum") or "")[:16]))

    lines = [
        "# 第三方声明与许可清单",
        "",
        "> 由 `python tools/notices.py` 生成，数据取自 `Cargo.lock` 与本机 registry 缓存：",
        "> 版本号、SPDX 表达式、`.crate` 的 SHA-256 都不是手抄（21 §11）。",
        "> 核心只收宽松许可；仓库内研究源码与候选二进制按各自来源和许可证单独列示，不代表它们已进入默认安装包。",
        "",
        "| 包 | 版本 | License | 仓库 | .crate SHA-256 | 分发 |",
        "|---|---|---|---|---|---|",
    ]
    for name, version, lic, repo, sha, _ck in rows:
        lines.append("| %s | %s | %s | %s | %s | bundled(允许随包) |" % (name, version, lic, repo, sha[:16] + ("..." if sha else "")))
    lines += ["", "## 非 crates.io 组件", "", "| 组件 | License | 来源 | 角色 |", "|---|---|---|---|"]
    for name, lic, url, role in EXTRA:
        lines.append("| %s | %s | %s | %s |" % (name, lic, url, role))
    srows, sproblems = source_archive_rows()
    lines += ["", "## 仓库内第三方源码归档", "", "| 组件 | 版本/提交 | License | 许可附加说明 | 来源 | 归档 | 锁定 SHA-256 | 本机实测 | 状态 |", "|---|---|---|---|---|---|---|---|---|"]
    for r in srows:
        actual = r["actual"] or "—"
        lines.append(
            "| %s | %s | %s | %s | %s | %s | %s | %s | %s |"
            % (
                r["name"], r["version"], r["license"], r["license_notes"], r["source"], r["artifact"],
                r["locked"][:20] + "…", actual[:20] + ("…" if actual != "—" else ""), r["status"],
            )
        )
    problems.extend(sproblems)
    nrows, nproblems = native_rows()
    lines += ["", "## native 与外部二进制的哈希锁（21 §8/§9，W3）", ""]
    lines += [
        "> 这张表不是抄来的：`tools/notices.py` 读 `third_party/SOURCE_AUDIT_LOCK.json` 里锁定的哈希，",
        "> 再对**本机那个文件**重算一遍 SHA-256。`match` 是实测相等；`absent-here` 是这台机器上没有那个文件，",
        "> 哈希仍是锁定的事实但没在这里实测过 —— 这两种状态必须分开写，混起来就是假证据。",
        "",
        "| 组件 | 版本 | License | 产物 | 锁定的 SHA-256 | 本机实测 | 状态 |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in nrows:
        actual = r["actual"] or "—"
        lines.append(
            "| %s | %s | %s | %s | %s | %s | %s |"
            % (
                r["component"], r["version"], r["license"], r["path"],
                r["locked"][:20] + "…", actual[:20] + ("…" if actual != "—" else ""),
                r["status"],
            )
        )
    problems.extend(nproblems)
    lines += [
        "",
        "## 更新策略",
        "",
        "- crates.io 依赖：锁 `Cargo.lock`，升级走一次完整的 `bash tools/check_all.sh --strict`；",
        "- native provider（libass 等）：不入库（体积与许可分发，21 §7），但每次换版本都要在 `SOURCE_AUDIT_LOCK.json` 里记 commit + 下载 URL + SHA-256，本文件的锁表随之重算；",
        "- 外部可执行文件（ffprobe/ffmpeg 已锁；mkvtoolnix 尚未接入，接的那天要同样锁哈希）：不自动更新，随版本锁定并记录 build 来源与配置（21 §8/§9）。",
        "",
    ]
    text = "\n".join(lines) + "\n"

    rc = 0
    # 清单过期也是一种不一致：门阶跑的是 --check，不重新生成文件，
    # 所以要确认每条锁定的哈希都真的出现在现存的 THIRD_PARTY_NOTICES.md 里。
    notices = ROOT / "THIRD_PARTY_NOTICES.md"
    if check and notices.is_file():
        body = notices.read_text(encoding="utf-8")
        for r in nrows:
            if r["locked"][:20] not in body:
                problems.append(
                    "清单过期：%s 的锁定哈希不在 THIRD_PARTY_NOTICES.md 里，重跑 python tools/notices.py"
                    % r["component"]
                )
        for r in srows:
            if r["locked"][:20] not in body:
                problems.append(
                    "清单过期：%s 的源码归档锁定哈希不在 THIRD_PARTY_NOTICES.md 里，重跑 python tools/notices.py"
                    % r["name"]
                )
    if problems:
        print("PROBLEMS (%d):" % len(problems))
        for p in problems[:40]:
            print("  " + p)
        rc = 1
    if not check:
        (ROOT / "THIRD_PARTY_NOTICES.md").write_text(text, encoding="utf-8")
    print("NOTICES %s：%d 个第三方包 + %d 个源码归档 + %d 条 native/二进制哈希锁，%d 个问题" % ("FAIL" if rc else "OK", len(rows), len(srows), len(nrows), len(problems)))
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
