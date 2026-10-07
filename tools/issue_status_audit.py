#!/usr/bin/env python3
"""问题清单与状态文档必须互相自洽（目标第 3 步）。

`docs/ISSUES.md` 是"这件事什么状态"的唯一口径，`docs/STATUS.md` §4 与
`docs/TRACEABILITY.md` 各节的"没做（不要当成已完成）"是"交付面到哪了"的口径。
两边各写各的就会撞出 AJ1 那类事：一边已经交付、另一边还写着没做。

两类矛盾分别检查：
* **开着的问题在状态文档里看不见** —— 读者以为交付面没有缺口；
* **已解决的问题还被列在"没做"里** —— 交付面被无端缩小（或旧断言没回头改）。

不做的事：不判断某条问题的处理对不对，那只看得懂文字的人能做。这里只查引用关系。
"""
import io
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8")

ROW = re.compile(
    # 状态格允许带限定语（`已解决（用注入替代真 provider）`、`待解决（等 E3 语料）`）。
    # 以前这里要求严格三选一，于是**带限定语的行整行不被解析**：本轮数下来有 39 行，
    # 包括 AG1/Z6/D6/F3/F5/F7-F11 —— 它们既没进缺口表，也没被"已解决却还列在没做里"那条
    # 检查看过一眼。放宽成"以三选一开头，后面可以带括号说明"。
    r"^\|\s*([A-Z]{1,2}\d+)\s*\|.*\|\s*(已解决|未解决|待解决)[^|]*\|\s*(P\d)\s*\|\s*$"
)
SECTION = re.compile(r"^##\s+([A-Z])\.\s")
# 所有"看起来是一行问题"的编号：§E 那张决定表只有 4 格（没有状态/优先级列），
# 它不是状态行，但**引用它不算断链**，所以引用检查用这个宽集合。
ANY_ROW_ID = re.compile(r"^\|\s*([A-Z]{1,3}\d{1,3})\s*\|", re.M)


def load(path):
    return io.open(path, encoding="utf-8").read()


def all_row_ids(text):
    """引用检查用的宽集合：包含 §E 那种没有状态列的决定表行。"""
    ids = set(ANY_ROW_ID.findall(text))
    # 表头与分隔行不是编号
    return {i for i in ids if i not in ("编号",)}


def issues(text):
    """返回 {编号: (状态, 优先级, 标题)}，只认表格行。"""
    out = {}
    for line in text.split("\n"):
        m = ROW.match(line)
        if not m:
            continue
        ident, status, prio = m.groups()
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        title = cells[1]
        out[ident] = (status, prio, title)
    return out


def open_section_text(status_text):
    """STATUS §4「明确没有的东西」整段。"""
    i = status_text.index("## 4.")
    j = status_text.index("\n## ", i + 5)
    return status_text[i:j]


def not_done_lines(text):
    """TRACEABILITY 里所有以「没做（不要当成已完成）」开头的段落。"""
    out = []
    for block in text.split("\n\n"):
        if block.startswith("没做"):
            out.append(block)
    return out


# 第四类检查：**引用必须指得到东西**。
# 代码注释里写"见 docs/ISSUES.md C14"，而 C14 这一行早在某次重排里没了 —— 下一个读代码的
# 人就会去找一条不存在的裁决，或者更糟：以为那个问题没人管过。本轮就是这么挖出 4 处断链
# （segment.rs 两处指 C14、palette.rs 两处指 C15），而前三类检查都看不见这个方向，
# 因为它们只看 ISSUES 与 STATUS/TRACEABILITY 之间的引用。
REF = re.compile(r"(?:ISSUES|ISSUES\.md)[^\n]{0,20}?([A-Z]{1,2}\d{1,3})(?![0-9])")
TEXT_EXT = (".rs", ".md", ".py", ".sh", ".toml", ".json", ".ps1")
SKIP_DIRS = {"target", ".git", "node_modules", "fixtures"}
# 本工具自己的注释里必须举例"断链长什么样"，那些是例子不是引用；写历史时也别把
# 「ISSUES + 一个不存在的编号」拼在一起（那正好是指针的形状）—— 见 ISSUES AK14 的记法约定。
SKIP_FILES = {"tools/issue_status_audit.py"}


def ref_walk(base):
    """树里所有 `ISSUES … 编号` 形态的引用；返回 (文件, 行号, 编号)。"""
    hits = []
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for fn in filenames:
            if not fn.endswith(TEXT_EXT):
                continue
            p = os.path.join(dirpath, fn)
            rel = p.replace(os.sep, "/").lstrip("./")
            if rel in SKIP_FILES:
                continue
            try:
                text = load(p)
            except (UnicodeDecodeError, OSError):
                continue
            for m in REF.finditer(text):
                line = text[: m.start()].count("\n") + 1
                hits.append((rel, line, m.group(1)))
    return hits


def refs_in(text, table_ids):
    """给定一段文本，返回其中指不到任何问题行的引用编号（用于自测）。"""
    return [m.group(1) for m in REF.finditer(text) if m.group(1) not in table_ids]


def dangling_references(table_ids, tree_dirs):
    bad = []
    for d in tree_dirs:
        for path, line, ident in ref_walk(d):
            if ident in table_ids:
                continue
            # 表格行尾的优先级列（`… | P2 |`）不是引用；只有当同一处既能匹配到 ISSUES
            # 关键词又正好落在优先级列上时才排掉，别的都算断链。
            bad.append((path, line, ident))
    return bad


def main():
    iss = load("docs/ISSUES.md")
    status = load("docs/STATUS.md")
    trace = load("docs/TRACEABILITY.md")

    table = issues(iss)
    if not table:
        print("没读到任何问题行 —— 检查正则与表格格式")
        return 1

    done_ids = {k for k, v in table.items() if v[0] == "已解决"}
    open_ids = {k: v for k, v in table.items() if v[0] != "已解决"}

    # `--gap-table`：把 STATUS §4 的「开着的缺口」表真正生成出来，而不是手抄一份。
    # 手抄的那张表上一轮就漂移过（AH5 已解决还被列在缺口里），所以让同一个解析器负责。
    if "--gap-table" in sys.argv:
        def sort_key(k):
            m = re.match(r"([A-Z]+)(\d+)", k)
            return (m.group(1), int(m.group(2)))

        rows = 0
        for ident in sorted(open_ids, key=sort_key):
            st, prio, title = open_ids[ident]
            one = title if len(title) <= 60 else title[:58] + "…"
            print("| {} | {} {} | {} |".format(ident, st, prio, one))
            rows += 1
        print("<!-- 本表由 tools/issue_status_audit.py --gap-table 生成，共 {} 行，勿手改 -->".format(rows))
        return 0

    visible = open_section_text(status) + "\n" + "\n".join(not_done_lines(trace)) + "\n" + status[: status.index("## 4.")]

    invisible = []
    for ident, (st, prio, title) in sorted(open_ids.items()):
        if ident not in visible:
            invisible.append((ident, st, prio, title))

    # 已解决却还出现在"没做"清单里：只认**引用**形态的编号。
    # 引用判据：编号出现在 `（` / `(` / `/` / `ISSUES ` 之后，且后面不紧跟汉字 ——
    # 这样"代价正是 AI5 已经不划算"那类历史叙述不会被当成"AI5 还开着"，
    # 而 `，P1）` 这种优先级标记也不会被误认成 P 节的 P1。
    stale = []
    narrowed = "\n".join([open_section_text(status), "\n".join(not_done_lines(trace))])
    cited = re.compile(r"(?:[（(/]|ISSUES\s)([A-Z]{1,2}[0-9]+)(?![一-鿿])")
    # 优先级标记长得和问题编号一样（P 节确实存在 P1..P11），所以只排掉"整对括号里就是一个
    # P+数字"这一种形状：`（P2）` 是优先级，`（AG8/AH5，P2）` 里的 P2 也是，
    # 但 `（P9）` 这种真编号在括号里单独出现时我们宁可放过而不是误杀 —— 全树扫下来没有这种写法。
    priority_only = re.compile(r"（[P]\d）")
    cited_here = set(cited.findall(narrowed))
    for m in priority_only.finditer(narrowed):
        token = m.group()[1:-1]
        # 只有当这个 token 不作为任何"引用形状"的一部分出现在别处时才剔掉
        rest = narrowed.replace(m.group(), "", 1)
        if token in done_ids and not re.search(r"(?:[（(/]|ISSUES\s)" + token + r"(?![一-鿿])", rest):
            cited_here.discard(token)
    for ident in sorted(done_ids & cited_here):
        ctx = [ln.strip()[:70] for ln in narrowed.split("\n") if ident in ln]
        stale.append((ident, ctx[:2]))

    print(f"问题总数 {len(table)}｜已解决 {len(done_ids)}｜开着 {len(open_ids)}")
    print("§4 + 各节『没做』里被引用到的编号数：{}".format(len(cited_here)))
    print("（引用判据见上面的注释：叙述性的旧事重提不算，优先级标记不算）")
    print()
    print("== 开着的问题在 STATUS/TRACEABILITY 里一处都没被点名（缺口没被标出来，软信号）：")
    for row in invisible:
        print("   {} [{} {}] {}".format(*row))
    if not invisible:
        print("   无")
    print()
    print("== 已解决却仍被列在『没做』清单里（AJ1 的那类矛盾）：")
    for ident, ctx in stale:
        print("   {}".format(ident))
        for c in ctx:
            print("      残句：{}".format(c))
    if not stale:
        print("   无")
    print()

    all_ids = all_row_ids(iss) | set(table)
    broken = dangling_references(all_ids, ["."])
    print("== 引用指不到问题行的断链（代码注释里写「见 ISSUES C14」而那一行不存在）：")
    for path, line, ident in sorted(broken):
        print("   {}:{} → {}".format(path, line, ident))
    if not broken:
        ref_total = len(ref_walk("."))
        print("   无（树里 {} 处 ISSUES 引用全部指得到行）".format(ref_total))
    print()
    print("（第四类是本轮新加的：前三类只看 ISSUES ↔ STATUS/TRACEABILITY，看不见代码注释指向"
          "已被重排掉的编号 —— 那样挖出的 4 处断链见 ISSUES AK14）")

    # 把关器自己要有牙：解析器坏了的表现是"零断链=通过"。
    if "--ref-selftest" in sys.argv:
        sample = (
            "见 docs/ISSUES.md Z9 那行；另有一处写着（docs/ISSUES.md Q77）是编的，"
            "还有一处 `ISSUES W42` 也是编的。"
        )
        caught = refs_in(sample, {"Z9"})
        assert caught == ["Q77", "W42"], "断链自测失败，抓到的是：{}".format(caught)
        # 反向：真编号不许被当成断链（否则第四类检查本身会造假红）
        assert refs_in("见 docs/ISSUES.md Z9", {"Z9"}) == [], "合法引用被误判"
        print("REF SELFTEST OK（一条假引用被抓住、一条真引用没被误伤）")

    return 0 if not invisible and not stale and not broken else 2


if __name__ == "__main__":
    sys.exit(main())
