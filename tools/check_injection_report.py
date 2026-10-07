#!/usr/bin/env python3
"""核对 `render --json` 报告里的 `font_injection` 段（17 §7）。

为什么不用 grep：门阶原先用 `grep '"font_injection":{"enabled":true'`，而 serde_json
把对象键按**字母序**排，给这段加一个 `embedded` 字段就把 `enabled` 挤到了第二位，
于是"功能正常、门阶变红"（ISSUES AK7）。字段顺序是实现细节，不是契约。
这里按语义核对，并且**缺字段一律报错**——"没查到"不等于"没问题"。
"""

import json
import re
import sys

# 门阶日志是 UTF-8；Windows 控制台默认 cp936，不归一会把中文结论写成乱码字节
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

SHA = re.compile(r"[0-9a-f]{64}")


def die(msg):
    print("CHECK FAIL: " + msg)
    return 1


def load_report(path):
    """CLI 的 JSON 报告与 libass 的 stderr 混在同一个日志里，只有那一行是报告。"""
    objs = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            s = line.strip()
            if s.startswith("{") and s.endswith("}"):
                try:
                    objs.append(json.loads(s))
                except json.JSONDecodeError:
                    pass
    if not objs:
        raise ValueError("没有一行能解析成 JSON——命令根本没产出报告，这不能当成「报告说没问题」")
    return objs[-1]


def need(d, key, path, what):
    if key not in d:
        raise ValueError("%s 缺少 %s（%s）" % (path, key, what))
    return d[key]


def check(path, expect_enabled, min_injected, min_embedded):
    report = load_report(path)
    if report.get("ok") is not True:
        raise ValueError("命令本身没成功：ok=%r，errors=%r" % (report.get("ok"), report.get("errors")))
    data = need(report, "data", path, "报告正文")
    inj = need(data, "font_injection", path, "注入台账")

    enabled = need(inj, "enabled", path, "这次到底开没开注入")
    if enabled is not expect_enabled:
        raise ValueError(
            "enabled=%r，但这次命令行%s该开注入" % (enabled, "应该" if expect_enabled else "不该")
        )

    requested = need(inj, "requested", path, "计划交出的 face 数")
    embedded = need(inj, "embedded", path, "其中内嵌字体数")
    from_library = need(inj, "from_library", path, "其中本机字体库数")
    if requested != embedded + from_library:
        raise ValueError(
            "requested=%r 不等于 embedded=%r + from_library=%r，台账自己都对不上"
            % (requested, embedded, from_library)
        )

    injected = need(inj, "injected", path, "实际交出的字节台账")
    if not isinstance(injected, list):
        raise ValueError("injected 不是列表：%r" % (injected,))

    # 没开注入就必须一个字节都没交出。这条是"报告不许说谎"的正面对手：
    # 只断言 enabled=false，而台账里还留着字节，等于开了注入却说不开。
    if not expect_enabled and injected:
        raise ValueError("enabled=false 但 injected 有 %d 项——报告与实际不符" % len(injected))
    if expect_enabled and len(injected) < min_injected:
        raise ValueError(
            "开了注入却只交出 %d 份（要求至少 %d 份）；fixture 里应当有这些 face"
            % (len(injected), min_injected)
        )
    if expect_enabled and len(injected) != requested:
        raise ValueError("计划 %r 份、实际交出 %d 份，中间没有失败可言" % (requested, len(injected)))
    # 内嵌那条通路必须真的被走到，否则这一阶只证明了"从本机字体库取字节"（AK5）
    if embedded < min_embedded:
        raise ValueError(
            "台账里内嵌字体只有 %r 份，要求至少 %d 份——`[Fonts]` 那条通路没被跑到"
            % (embedded, min_embedded)
        )

    shas = []
    for i, e in enumerate(injected):
        name = need(e, "name", path, "第 %d 项的字体名" % i)
        size = need(e, "bytes", path, "第 %d 项的字节数" % i)
        sha = need(e, "sha256", path, "第 %d 项的内容哈希" % i)
        if not name:
            raise ValueError("第 %d 项没有名字，无法证明这份字节被按名字交给了谁" % i)
        if not isinstance(size, int) or size <= 0:
            raise ValueError("第 %d 项（%s）字节数 %r 不合理" % (i, name, size))
        if not isinstance(sha, str) or SHA.fullmatch(sha) is None:
            raise ValueError("第 %d 项（%s）的 sha256 %r 不是 64 位小写十六进制" % (i, name, sha))
        shas.append(sha)
    if len(set(shas)) != len(shas):
        raise ValueError("台账里有同一份字节被记录了两次：%r" % (shas,))

    env = need(data, "font_env", path, "字体环境身份")
    digest = need(env, "digest", path, "渲染身份摘要")
    if not isinstance(digest, str) or SHA.fullmatch(digest) is None:
        raise ValueError("render_identity 摘要 %r 不是 SHA-256" % (digest,))

    print(
        "CHECK OK: enabled=%s requested=%d embedded=%d from_library=%d injected=%d digest=%s"
        % (enabled, requested, embedded, from_library, len(injected), digest[:12])
    )
    return 0


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        print(__doc__)
        print("用法：python tools/check_injection_report.py <日志> <true|false> <最少注入份数> [最少内嵌份数]")
        sys.exit(2)
    try:
        sys.exit(
            check(
                sys.argv[1],
                sys.argv[2] == "true",
                int(sys.argv[3]),
                int(sys.argv[4]) if len(sys.argv) == 5 else 0,
            )
        )
    except (ValueError, OSError) as e:
        sys.exit(die(str(e)))
