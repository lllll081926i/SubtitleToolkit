#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成 THIRD_PARTY_NOTICES.md 并按 21 §1/§11 校验许可证白名单。

21 §11 要求：每个依赖记录 version、URL、license、hash、bundled/external、更新策略。
这份脚本从 `Cargo.lock` + 本机注册表缓存里取真实数据（不是手抄），并做两件事：

1. 写 `THIRD_PARTY_NOTICES.md`；
2. 任何不在 `deny.toml` 的 `[licenses].allow` 里的 SPDX 表达式都让脚本退出 1
   —— 也就是说这条门阶会在有人偷偷引入 GPL 依赖时变红，而不是等发布前才发现。

用法：python tools/notices.py [--check] [--fetch]
  --check  只校验不写文件
  --fetch  把本机缺的 .crate 归档下载下来再校验（平台专属依赖在 Windows 上从不下）
"""
from __future__ import annotations

import hashlib
import os
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def workspace_version() -> str:
    """从根 Cargo.toml 读版本，不手抄。

    SBOM 里的产品版本手抄一份就会在下次发版时与二进制不一致，
    而 "这份 SBOM 描述的是哪个构建" 是它唯一的用途。
    读不到时返回 "unknown" 而不是编一个：读者应当能看出这个字段没填对。
    """
    try:
        data = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        v = data.get("workspace", {}).get("package", {}).get("version")
        return str(v) if v else "unknown"
    except (OSError, tomllib.TOMLDecodeError):
        return "unknown"


APP_VERSION = workspace_version()
# 门阶把这条的输出重定向进日志文件；Windows 控制台默认 cp936，会把中文结论写成非 UTF-8 字节，
# 于是 `target/gate/notices.log` 既不好读也没法 diff（与 tools/doc_claims_check.py 同一个处理）。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
CARGO_HOME = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))

# 21 §1 之外的组件：不是 crates.io 依赖，但会随包/随验证出现，必须单独交代
EXTRA = [
    ("libass", "ISC", "https://github.com/libass/libass",
     "随便携包与安装包分发；逐 DLL 来源、哈希、包元数据和许可证正文见包内 licenses/LIBASS_RUNTIME_LOCK.json 与 licenses/msys2/"),
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
    """读 license / repository（发布出去的就是这份声明）。

    优先读已解压的源码；找不到时回退到 `.crate` 归档内的 `Cargo.toml`。
    为什么要回退：Windows x64 只编译一部分 crate，**平台专属的传递依赖**
    （Android/iOS/Linux/wasm）会被下载但从不解压。只看 `registry/src`
    会把它们误报成"读不到 license"，而许可其实就在归档里。
    """
    manifest = _manifest_from_src(name, version)
    text = None
    if manifest is not None:
        text = manifest.read_text(encoding="utf-8", errors="replace")
    else:
        text = _manifest_from_archive(name, version)
    if text is None:
        return (None, None)
    m = re.search(r'(?m)^license\s*=\s*"([^"]+)"', text)
    repo = re.search(r'(?m)^repository\s*=\s*"([^"]+)"', text)
    lic = m.group(1) if m else None
    if lic is None:
        expr = re.search(r'(?m)^license-file\s*=\s*"([^"]+)"', text)
        if expr:
            lic = "NONSTANDARD(license-file: " + expr.group(1) + ")"
    return (lic, repo.group(1) if repo else None)


def _manifest_from_src(name: str, version: str) -> Path | None:
    manifests = sorted((CARGO_HOME / "registry" / "src").glob(f"*/{name}-{version}/Cargo.toml"))
    return manifests[0] if manifests else None


def _manifest_from_archive(name: str, version: str) -> str | None:
    """从 `.crate`（gzip 包住的 tar）里读取 `Cargo.toml` 文本。

    用标准库的 gzip+tarfile，不引入第三方依赖。读取失败返回 None，
    调用方照旧报"读不到 license"，不会静默当成已授权。
    """
    archives = _crate_archives(name, version)
    if not archives:
        return None
    import gzip
    import tarfile

    try:
        with gzip.open(archives[0], "rb") as gz, tarfile.open(fileobj=gz, mode="r:") as tar:
            member = tar.getmember(f"{name}-{version}/Cargo.toml")
            fh = tar.extractfile(member)
            if fh is None:
                return None
            return fh.read().decode("utf-8", errors="replace")
    except (OSError, KeyError, tarfile.TarError):
        return None


def _crate_archives(name: str, version: str) -> list[Path]:
    """cargo 把归档放在 `cache/<registry>-<hash>/name-version.crate` 下。

    两个位置都查：以 registry 子目录为主，兼容直接放在 `cache/` 根下的情况
    （`fetch_missing()` 就写在那里）。
    """
    root = CARGO_HOME / "registry" / "cache"
    found = sorted(root.glob(f"*/{name}-{version}.crate"))
    direct = root / f"{name}-{version}.crate"
    if direct.is_file():
        found.append(direct)
    return found


# sparse index 的下载模板：{crate}/{version}/download
REGISTRY_DL_BASES = (
    "https://rsproxy.cn/api/v1/crates",
    "https://static.crates.io/crates",
)


def fetch_missing(names: list[tuple[str, str]]) -> int:
    """把本机缺的 `.crate` 归档下来，让许可门阶能真的检查它们。

    为什么要这一步：Windows x64 只编译一部分 crate，**平台专属的传递依赖**
    （Android/iOS/Linux/wasm）会被写进 `Cargo.lock` 但从不下载。它们的许可
    就只存在于归档里，不取下来就只能报"读不到"——而"读不到"跟"没许可证"
    是两回事，不能混。

    返回下载失败条数；失败不静默跳过。
    """
    import urllib.error
    import urllib.request

    cache_root = CARGO_HOME / "registry" / "cache"
    cache_root.mkdir(parents=True, exist_ok=True)
    failed = 0
    for name, version in names:
        if _crate_archives(name, version):
            continue
        dest = cache_root / f"{name}-{version}.crate"
        got = False
        for base in REGISTRY_DL_BASES:
            url = f"{base}/{name}/{version}/download"
            try:
                with urllib.request.urlopen(url, timeout=60) as resp:
                    data = resp.read()
            except (urllib.error.URLError, OSError, TimeoutError):
                continue
            if data[:2] == b"\x1f\x8b":  # gzip 魔数：确认拿到的是归档而不是错误页
                dest.write_bytes(data)
                got = True
                break
        if not got:
            print(f"  下载失败：{name} {version}")
            failed += 1
    return failed


def crate_sha256(name: str, version: str) -> str | None:
    archives = _crate_archives(name, version)
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
    """把 SPDX 表达式拆成单个许可标识（用于清单展示）。

    现实里三种写法都出现过：`MIT OR Apache-2.0`、`MIT/Apache-2.0`（老式斜杠=OR）、
    `(MIT OR Apache-2.0) AND Unicode-3.0`。括号与空白都是噪声，剥掉再比。

    注意：本函数只是展开，**不做合规判定**。合规判定要用 `expr_allowed()`，
    因为 `A OR B` 只需一侧在白名单里就算过，“逐个 token 都必须白名单”是错的读法。
    """
    parts = re.split(r"(?i)\s+(?:OR|AND)\s+|\s*\|\|\s*|\s*/\s*", expr.strip())
    out = []
    for p in parts:
        token = p.strip(" ()[]{},").strip()
        if token:
            out.append(token)
    return out


def expr_allowed(expr: str, allow: set[str]) -> tuple[bool, str]:
    """按 SPDX 语义判定表达式是否被白名单接纳。返回（是否通过，拒绝原因）。

    规则（SPDX 2.x）：
    * `A OR B`：任一侧在白名单内即可（我们可自由选那一侧）；
    * `A AND B`：两侧都必须白名单，否则义务叠加后不满足；
    * 单个 token：必须在白名单内。

    为什么要专门做这件事：把 `Apache-2.0 OR GPL-2.0-only` 拆开后逐个 token 要求
    白名单，会把它误报成 GPL 污染 —— 而它实际上可以按 Apache-2.0 使用。
    反过来，`MPL-2.0` 这种没有宽松选项的必须照实报。
    """

    def _parse(s: str) -> tuple[bool, str]:
        s = s.strip()
        # 去一层最外层平衡括号
        if s.startswith("(") and s.endswith(")"):
            inner = s[1:-1]
            depth = 0
            balanced = True
            for i, ch in enumerate(inner):
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth < 0:
                        balanced = False
                        break
            if balanced and depth == 0:
                return _parse(inner)
        # OR 优先级低于 AND：先拆 OR
        or_parts = _top_split(s, "OR")
        if len(or_parts) > 1:
            reasons = []
            for p in or_parts:
                ok, why = _parse(p)
                if ok:
                    return (True, "")
                reasons.append(why)
            return (False, " 与 ".join(reasons))
        and_parts = _top_split(s, "AND")
        if len(and_parts) > 1:
            for p in and_parts:
                ok, why = _parse(p)
                if not ok:
                    return (False, why)
            return (True, "")
        token = s.strip(" ()[]{},").strip()
        if token in allow:
            return (True, "")
        return (False, f"许可 {token!r} 不在 21 §1 白名单内")

    return _parse(expr)


def _top_split(expr: str, op: str) -> list[str]:
    """在括号深度 0 处按操作符拆分。

    除了关键字，还要处理老式斜杠写法：`MIT/Apache-2.0` 与 `Apache-2.0/MIT` 在
    整条生态系统里常见，SPDX 官方已弃用但包元数据里仍大量存在，斜杠等价于 OR。
    """
    out: list[str] = []
    depth = 0
    buf = []
    i = 0
    n = len(expr)
    while i < n:
        ch = expr[i]
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if depth == 0:
            rest = expr[i:]
            m = re.match(rf"(?i)^\s+{op}\s+", rest)
            if m:
                out.append("".join(buf))
                buf = []
                i += m.end()
                continue
            # 老式斜杠只当 OR：仅当拆的是 OR 且在深度 0 时生效
            if op == "OR" and ch == "/":
                out.append("".join(buf))
                buf = []
                i += 1
                continue
        buf.append(ch)
        i += 1
    out.append("".join(buf))
    return [p for p in out if p.strip()]


def main(argv: list[str]) -> int:
    check = "--check" in argv
    fetch = "--fetch" in argv
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        print("FAIL: 没有 Cargo.lock（复现能力要求它入库）")
        return 1
    allow = allowed_licenses()
    if not allow:
        print("FAIL: deny.toml 的 [licenses].allow 是空的")
        return 1

    # 先把本机缺的归档补齐（只补下载，不改任何声明）：平台专属传递依赖在 Windows
    # 上从不被下载，不补就只能报"读不到"，而那是环境事实不是许可事实。
    registry_pkgs = [
        (p["name"], p.get("version", ""))
        for p in parse_lock(lock)
        if "registry+" in p.get("source", "")
    ]
    absent = [(n, v) for (n, v) in registry_pkgs if not _crate_archives(n, v) and _manifest_from_src(n, v) is None]
    if absent:
        if fetch:
            print("本机缺 %d 个包，按 --fetch 下载归档…" % len(absent))
            fetch_missing(absent)
        else:
            print("提示：本机缺 %d 个包（平台专属依赖），未下载。加 --fetch 可下载后校验。" % len(absent))

    rows = []
    problems = []
    for name, version in sorted(registry_pkgs, key=lambda t: (t[0], t[1])):
        lic, repo = crate_license(name, version)
        sha = crate_sha256(name, version)
        if lic is None:
            problems.append(
                "%s %s: 读不到 license 字段（本机既无解压源码也无 .crate 归档；用 --fetch 下载后重试）"
                % (name, version)
            )
            lic = "UNKNOWN"
        else:
            ok, why = expr_allowed(lic, allow)
            if not ok:
                problems.append("%s %s: %s" % (name, version, why))
        pkg = next(p for p in parse_lock(lock) if p["name"] == name and p.get("version", "") == version)
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
        "- crates.io 依赖：锁 `Cargo.lock`；升级运行受影响的 Rust 行为测试与工作区检查，可选综合门阶不替代真实依赖验证；",
        "- native provider：libass 运行库随便携包与安装包分发；逐 DLL 来源、版本、SHA-256 和许可证材料由 `third_party/LIBASS_RUNTIME_LOCK.json` 锁定，构建时逐项校验；",
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
        # SBOM 同样会过期：依赖增删后不重生成，它就描述了一个不存在的构建。
        # 只检查“存在 + 能解析 + 组件数与当前依赖对得上”，不比整份内容：
        # 时间戳每次生成都会变，逐字节比对会让门阶永远红。
        sbom = ROOT / "sbom.cdx.json"
        if not sbom.is_file():
            problems.append("缺少 sbom.cdx.json：28 §2 要求 checksums + SBOM + third-party notices")
        else:
            import json

            try:
                doc = json.loads(sbom.read_text(encoding="utf-8"))
            except (OSError, ValueError) as e:
                problems.append("sbom.cdx.json 解析失败：%s" % e)
            else:
                got = len(doc.get("components", []))
                want = len(rows) + len(srows) + len(nrows) + len(EXTRA)
                if got != want:
                    problems.append(
                        "SBOM 过期：有 %d 个组件，当前依赖树应该是 %d 个，重跑 python tools/notices.py"
                        % (got, want)
                    )
                if doc.get("metadata", {}).get("component", {}).get("version") != APP_VERSION:
                    problems.append(
                        "SBOM 里的产品版本与 Cargo.toml 不一致（SBOM %s ｜ 仓库 %s）"
                        % (doc.get("metadata", {}).get("component", {}).get("version"), APP_VERSION)
                    )
    if problems:
        print("PROBLEMS (%d):" % len(problems))
        for p in problems[:40]:
            print("  " + p)
        rc = 1
    if not check:
        (ROOT / "THIRD_PARTY_NOTICES.md").write_text(text, encoding="utf-8")
        write_sbom(rows, srows, nrows)
    print("NOTICES %s：%d 个第三方包 + %d 个源码归档 + %d 条 native/二进制哈希锁，%d 个问题" % ("FAIL" if rc else "OK", len(rows), len(srows), len(nrows), len(problems)))
    return rc


def sbom_document(rows: list, srows: list[dict], nrows: list[dict]) -> dict:
    """按 28 §2 的 `checksums + SBOM + third-party notices` 产出 CycloneDX 1.5 文档。

    为什么挑 CycloneDX 而不是 SPDX：它原生表达“组件 + 哈希 + 许可证”这三件本仓库
    真正持有的事实（21 §11 要求的正是这三样），而 SPDX 的文档模型更偏静态许可声明。
    选它的另一个理由是字段少且直白——能用一份自己的代码写对，就不必引第三方依赖。

    三条纪律：

    1. **只写我们真的知道的事**。拿不到 license 的写 `UNKNOWN` 而不是猜一个；
       拿不到 `.crate` 哈希的就不写哈希字段。SBOM 的价值在于可核对，编进去的值会毁掉它。
    2. **`dev-only` 与 `bundled` 必须分开**。开发期工具（cargo-deny、vcpkg）不进安装包，
       把它们列成 runtime 组件会让下游误判分发义务。这与 THIRD_PARTY_NOTICES.md 的口径一致。
    3. **native/二进制产物带实测状态**。`absent-here` 是“这台机器上没有那个文件”，
       与 `match` 分开写：混起来就是拿未实测的值当已核对。
    """
    import datetime

    components: list[dict] = []

    def spdx_ids(expr: str) -> list[str]:
        # SBOM 里许可证用 SPDX id 列表；表达式的 OR/AND 结构在 notices 表里已经写明，
        # 这里展开成 id 集合，不重复表达组合语义（那会与 notices 表形成两个据）。
        out = []
        for part in re.split(r"[()\s]+|\bOR\b|\bAND\b|\bWITH\b|/", expr):
            p = part.strip()
            if p and p not in out and p not in {"OR", "AND", "WITH"}:
                out.append(p)
        return out or ["UNKNOWN"]

    for name, version, lic, repo, sha, _ck in rows:
        c: dict = {
            "type": "library",
            "name": name,
            "version": version or "unknown",
            "scope": "required",
            "licenses": [{"license": {"id": i}} if i != "UNKNOWN" else {"license": {"name": "UNKNOWN"}}
                         for i in spdx_ids(lic)],
        }
        if sha:
            c["hashes"] = [{"alg": "SHA-256", "content": sha}]
        if repo:
            c["externalReferences"] = [{"type": "vcs", "url": repo}]
        components.append(c)

    for r in srows:
        c = {
            "type": "library",
            "name": r["name"],
            "version": r["version"] or "unknown",
            # 源码归档是研究材料，不是默认安装包依赖；标 excluded 而不是 required
            "scope": "excluded",
            "licenses": [{"license": {"id": i}} for i in spdx_ids(r["license"])],
            "properties": [
                {"name": "stk:role", "value": "source-archive(研究材料，不入默认安装包)"},
                {"name": "stk:artifact", "value": r["artifact"]},
                {"name": "stk:hash-status", "value": r["status"]},
            ],
        }
        if r["locked"]:
            c["hashes"] = [{"alg": "SHA-256", "content": r["locked"]}]
        if r["source"]:
            c["externalReferences"] = [{"type": "vcs", "url": r["source"]}]
        components.append(c)

    for r in nrows:
        c = {
            "type": "library",
            "name": r["component"],
            "version": r["version"] or "unknown",
            "scope": "required",
            "licenses": [{"license": {"id": i}} for i in spdx_ids(r["license"])],
            "properties": [
                {"name": "stk:artifact-path", "value": r["path"]},
                # 实测状态分开写：absent-here 不等于已核对
                {"name": "stk:hash-status", "value": r["status"]},
                {"name": "stk:measured-sha256", "value": r["actual"] or "(本机无此文件)"},
            ],
        }
        if r["locked"]:
            c["hashes"] = [{"alg": "SHA-256", "content": r["locked"]}]
        components.append(c)

    for name, lic, url, role in EXTRA:
        components.append(
            {
                "type": "library",
                "name": name,
                "version": "n/a",
                # dev-only 的明确标 excluded：下游据此判断分发义务
                "scope": "excluded" if "dev-only" in role else "required",
                "licenses": [{"license": {"id": i}} for i in spdx_ids(lic)],
                "externalReferences": [{"type": "website", "url": url}],
                "properties": [{"name": "stk:role", "value": role}],
            }
        )

    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "timestamp": datetime.datetime.now(datetime.timezone.utc)
            .replace(microsecond=0)
            .isoformat()
            .replace("+00:00", "Z"),
            "tools": [{"vendor": "SubtitleToolkit", "name": "tools/notices.py", "version": "1"}],
            "component": {"type": "application", "name": "SubtitleToolkit", "version": APP_VERSION},
            "properties": [
                {
                    "name": "stk:scope-note",
                    "value": "required=随安装包分发；excluded=开发期工具或研究材料，不随包；"
                    "native 产物的 stk:hash-status 是**本机实测**结果，absent-here 不等于已核对",
                }
            ],
        },
        # CycloneDX 用 urn:uuid 或 serialNumber 标识一次生成；
        # 用内容派生的稳定串而不是随机 uuid：同一份依赖树两次生成应当得到同一标识。
        "serialNumber": "urn:stk:sbom:" + file_sha256(ROOT / "Cargo.lock")[:32],
        "components": components,
    }


def write_sbom(rows: list, srows: list[dict], nrows: list[dict]) -> None:
    import json

    doc = sbom_document(rows, srows, nrows)
    out = ROOT / "sbom.cdx.json"
    out.write_text(json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
