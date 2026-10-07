"""盘点每条校验规则的**双向用例**覆盖（ISSUES AK17；教训来自 AK16）。

`PGS.TML.05` 曾经把 fps 当成秒数换算，偏严地误伤合法文件，却长期没人发现：
它唯一的用例在 bug 下也会报 warning —— 为错误的理由通过。推广这一问法：
一条规则如果只有"该报"的用例，算偏严也看不出来；只有"不该报"的用例，
规则失效同样看不出来。本脚本按 check id 把测试里的引用分成四类列出来。

它是**盘点工具，不是门槛**：RC 恒为 0。"测试没点名 id"不等于"没覆盖"
（可能由 severity/no_blocking_findings 级的断言间接覆盖），但那种间接覆盖
分不清"规则失效"和"规则被间接覆盖" —— 正是 AK16 的形状。所以这里只把
事实量出来，交给读的人决定补哪条用例。
"""
import io
import re
import os

# 只有正例（该报）→ 规则算偏严也能长期活着；只有反例（不该报）→ 规则失效没人知道。
CHECK_ID = re.compile(r"r\.add\(\s*\"([A-Z]{2,4}\.[A-Z]{2,4}\.\d{2})\"|\"([A-Z]{2,4}\.[A-Z]{2,4}\.\d{2})\"", re.S)
SIMPLE = re.compile(r"\"([A-Z]{2,6}\.[A-Z]{2,6}\.\d{2,3})\"")
POS = re.compile(r"has_check\(\s*\"([^\"]+)\"|check\s*==\s*\"([^\"]+)\"|find\(\|f\| f\.check == \"([^\"]+)\"")
NEG = re.compile(r"!r\.has_check\(\s*\"([^\"]+)\"|!\w+\.has_check\(\s*\"([^\"]+)\"|is_none")

files = []
for d, dirs, fs in os.walk("crates"):
    dirs[:] = [x for x in dirs if x != "target"]
    for f in fs:
        if f.endswith(".rs"):
            files.append(os.path.join(d, f))

# 1) 规则清单：谁定义的（r.add("X")），以及它在哪个 crate
rules = {}
for p in files:
    t = io.open(p, encoding="utf-8", errors="replace").read()
    for m in SIMPLE.finditer(t):
        cid = m.group(1)
        # 只收"像检查点 id"的串：有点号分层
        rules.setdefault(cid, set()).add(p.replace("\\", "/"))

# 2) 用例方向：在测试里提到这条 id 的地方，是"报出来"还是"不该报"
pos, neg = {}, {}
for p in files:
    t = io.open(p, encoding="utf-8", errors="replace").read()
    # 逐个 assert/期望语句地扫
    # 否定式的写法有三种形状：!r.has_check(X)、!structure(&s).has_check(X)、
    # !timeline(...).has_check(X) —— 只要 has_check 前同一个断言里出现 `!` 就算"不该报"。
    for m in re.finditer(r"(assert!\s*\(\s*)?(!)?\s*[\w&:]+\s*\.has_check\(\s*\"([^\"]+)\"", t):
        cid = m.group(3)
        (neg if m.group(2) else pos).setdefault(cid, set()).add(p.replace("\\", "/"))
    for m in re.finditer(r"assert!\s*\(\s*![\s\S]{0,160}?has_check\(\s*\"([^\"]+)\"", t):
        neg.setdefault(m.group(1), set()).add(p.replace("\\", "/"))
    for m in re.finditer(r"check\s*==\s*\"([^\"]+)\"", t):
        pos.setdefault(m.group(1), set()).add(p.replace("\\", "/"))
    for m in re.finditer(r"find\(\s*\|\s*\w+\s*\|\s*\w+\.check\s*==\s*\"([^\"]+)\"", t):
        pos.setdefault(m.group(1), set()).add(p.replace("\\", "/"))
    for m in re.finditer(r"expect_err[\s\S]{0,200}?code_string\(\),\s*\"([^\"]+)\"", t):
        pos.setdefault(m.group(1), set()).add(p.replace("\\", "/"))

ids = sorted(k for k in rules if "." in k and k[0].isupper() and len(k) >= 6)
print("扫到的检查点 id：%d 条" % len(ids))
only_pos, only_neg, both, none = [], [], [], []
for cid in ids:
    p, n = cid in pos, cid in neg
    (both if p and n else only_pos if p else only_neg if n else none).append(cid)

def show(title, lst):
    print("\n%s（%d 条）" % (title, len(lst)))
    for c in lst:
        print("   ", c, "->", ", ".join(sorted(rules.get(c, ["?"]))[:2]))

show("只有『该报』的用例（偏严的 bug 能长期活着 —— AK16 那一类）", only_pos)
show("只有『不该报』的用例（规则失效没人知道）", only_neg)
show("两个方向都有", both)
show("测试里一次都没提到", none)
print(
    "\n小结：%d 该报单向 / %d 不该报单向 / %d 双向 / %d 未覆盖"
    % (len(only_pos), len(only_neg), len(both), len(none))
)
