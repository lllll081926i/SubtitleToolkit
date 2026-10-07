#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""用 pysubs2 做字幕语义的第三方交叉验证（AT-ASS-001 / 24 §QA 的"外部实现参与"）。

Rust 侧的 parse→render 往返只能证明"我们跟自己一致"。pysubs2（MIT）是独立实现，
它对字段顺序、时间格式、默认值的取舍不同，正好用来抓"我们自己解释错但内部自洽"的缺陷
（Pillow 在 PNG 层就是这么抓到 CRC 的，见 ISSUES M2）。

sidecar JSON 描述的是**语义**，不是字节：
{
  "schema": 1,
  "playres": [w, h] | null,
  "sbs": "yes"|"no"|null,
  "title": str | null,
  "styles": [{"name","fontname","fontsize","alignment","margin_l","margin_r","margin_v"}],
  "events": [{"start_ms","end_ms","layer","style","name","margin_l","margin_r","margin_v","effect","text"}]
}

用法：
  python tools/subs_pysubs2_check.py verify <dir>   # 核对 dir 下 ours_*.ass/.srt 与 ours_*.json
  python tools/subs_pysubs2_check.py emit <dir>     # 由 pysubs2 产出 ref_*.ass/.srt + ref_*.json
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import pysubs2

SCHEMA = 1

TIME_RE = re.compile(
    r"(\d+):(\d+):(\d+)[,.](\d+)\s*-->\s*(\d+):(\d+):(\d+)[,.](\d+)"
)


def fail(msg):
    print("Pysubs2Check FAIL: " + msg)
    sys.exit(1)


def num(value):
    """pysubs2 把 Fontsize/对齐/边距读成 int 或 float（`60` 变成 `60.0`）。

    那是它的内部表示，不是语义差异，统一规范成整数串再比。
    """
    if isinstance(value, bool):
        return str(int(value))
    if isinstance(value, float):
        if value.is_integer():
            return str(int(value))
        return repr(value)
    return str(value)


def to_ms(h, mi, s, frac):
    return ((int(h) * 60 + int(mi)) * 60 + int(s)) * 1000 + int(frac)


def srt_model_from_file(path):
    """SRT 的期望必须取自文件里真正写的内容。

    pysubs2 载入 SRT 时会把 `<i>` 转成 ASS 的 `{\\i1}` 内部表示；拿那个当期望，
    就会把"表示选择"冒充成"语义差异"。
    """
    text = path.read_text(encoding="utf-8-sig")
    blocks = [b for b in re.split(r"\r?\n\r?\n", text.strip()) if b.strip()]
    events = []
    for block in blocks:
        lines = block.replace("\r", "\n").split("\n")
        if len(lines) < 2:
            continue
        m = TIME_RE.match(lines[1].strip())
        if not m:
            continue
        g = m.groups()
        events.append({
            "start_ms": to_ms(g[0], g[1], g[2], g[3]),
            "end_ms": to_ms(g[4], g[5], g[6], g[7]),
            "layer": "0",
            "style": "",
            "name": "",
            "margin_l": "0",
            "margin_r": "0",
            "margin_v": "0",
            "effect": "",
            "text": "\n".join(lines[2:]),
        })
    return {
        "schema": SCHEMA,
        "playres": None,
        "sbs": None,
        "title": None,
        "styles": [],
        "events": events,
    }


def model_of(sub):
    playres = None
    if sub.info.get("PlayResX") is not None and sub.info.get("PlayResY") is not None:
        playres = [int(sub.info["PlayResX"]), int(sub.info["PlayResY"])]
    sbs_raw = sub.info.get("ScaledBorderAndShadow")
    styles = []
    for name, st in sub.styles.items():
        styles.append({
            "name": name,
            "fontname": st.fontname,
            "fontsize": num(st.fontsize),
            "alignment": num(st.alignment),
            "margin_l": num(st.marginl),
            "margin_r": num(st.marginr),
            "margin_v": num(st.marginv),
        })
    events = []
    for line in sub:
        events.append({
            "start_ms": int(line.start),
            "end_ms": int(line.end),
            "layer": num(line.layer),
            "style": line.style or "",
            "name": line.name or "",
            "margin_l": num(line.marginl) if line.marginl is not None else "0",
            "margin_r": num(line.marginr) if line.marginr is not None else "0",
            "margin_v": num(line.marginv) if line.marginv is not None else "0",
            "effect": line.effect or "",
            "text": str(line.text),
        })
    return {
        "schema": SCHEMA,
        "playres": playres,
        "sbs": (sbs_raw.lower() if isinstance(sbs_raw, str) else None),
        "title": sub.info.get("Title"),
        "styles": sorted(styles, key=lambda d: d["name"]),
        "events": events,
    }


def compare(actual, expected, label):
    if actual["schema"] != expected.get("schema", SCHEMA):
        fail(label + ": schema 不匹配")
    for key in ("playres", "sbs", "title"):
        if actual[key] != expected.get(key):
            fail("%s: %s 不符 actual=%r expected=%r" % (label, key, actual[key], expected.get(key)))
    want_styles = {d["name"]: d for d in expected["styles"]}
    for st in actual["styles"]:
        want = want_styles.get(st["name"])
        if want is None:
            fail("%s: pysubs2 读出样式 %r，期望里没有（期望 %r）" % (label, st["name"], sorted(want_styles)))
        for field in ("fontname", "fontsize", "alignment", "margin_l", "margin_r", "margin_v"):
            if st[field] != want[field]:
                fail("%s: 样式 %s.%s 不符 actual=%r expected=%r"
                     % (label, st["name"], field, st[field], want[field]))
    if len(actual["events"]) != len(expected["events"]):
        fail("%s: 事件数不符 actual=%d expected=%d"
             % (label, len(actual["events"]), len(expected["events"])))
    for i, (got, want) in enumerate(zip(actual["events"], expected["events"])):
        for field in ("start_ms", "end_ms", "layer", "style", "name", "margin_l",
                      "margin_r", "margin_v", "effect", "text"):
            if got[field] != want[field]:
                fail("%s: events[%d].%s 不符 actual=%r expected=%r"
                     % (label, i, field, got[field], want[field]))


def load_expected(path):
    side = path.with_suffix(".json")
    if not side.is_file():
        fail("缺少期望文件 " + side.name)
    return json.loads(side.read_text(encoding="utf-8"))


def verify(directory):
    paths = sorted(list(directory.glob("ours_*.ass")) + list(directory.glob("ours_*.srt")))
    if not paths:
        fail(str(directory) + " 里没有 ours_*.ass/.srt")
    for path in paths:
        expected = load_expected(path)
        fmt = "srt" if path.suffix == ".srt" else "ass"
        try:
            # pysubs2 必须先能打开：这才是"第三方读得懂"
            loaded = pysubs2.load(str(path), format=fmt)
        except Exception as exc:  # noqa: BLE001
            fail("pysubs2 打不开 %s: %r" % (path.name, exc))
        actual = srt_model_from_file(path) if fmt == "srt" else model_of(loaded)
        compare(actual, expected, path.name)
    print("VERIFY OK %d" % len(paths))
    return len(paths)


def emit(directory):
    directory.mkdir(parents=True, exist_ok=True)
    made = [0]

    def write(name, sub):
        path = directory / name
        fmt = "ass" if path.suffix == ".ass" else "srt"
        sub.save(str(path), format=fmt)
        if fmt == "srt":
            model = srt_model_from_file(path)
        else:
            model = model_of(pysubs2.load(str(path), format=fmt))
        path.with_suffix(".json").write_text(
            json.dumps(model, ensure_ascii=False, indent=1), encoding="utf-8"
        )
        made[0] += 1

    rich = pysubs2.SSAFile()
    rich.info["PlayResX"] = "1920"
    rich.info["PlayResY"] = "1080"
    rich.info["ScaledBorderAndShadow"] = "yes"
    rich.info["Title"] = "cross-check"
    st = pysubs2.SSAStyle()
    st.fontname = "Arial"
    st.fontsize = 60
    st.alignment = 2
    st.marginl = 50
    st.marginr = 50
    st.marginv = 60
    st.primarycolor = pysubs2.Color(255, 255, 255)
    rich.styles["Default"] = st
    st2 = pysubs2.SSAStyle()
    st2.fontname = "Noto Sans CJK SC"
    st2.fontsize = 48
    st2.alignment = 8
    st2.marginl = 10
    st2.marginr = 10
    st2.marginv = 20
    rich.styles["Speaker"] = st2
    rich.append(pysubs2.SSAEvent(start=1000, end=3000, style="Default", layer=0,
                                 text=r"{\an5\pos(960,540)}第一行 {\i1}斜体{\i0} 尾巴"))
    rich.append(pysubs2.SSAEvent(start=4010, end=6000, style="Speaker", layer=1,
                                 marginv=30, name="A", effect="banner",
                                 text="两句\n第二行"))
    write("ref_rich.ass", rich)

    plain = pysubs2.SSAFile()
    plain.append(pysubs2.SSAEvent(start=0, end=1500, text="plain"))
    plain.append(pysubs2.SSAEvent(start=2000, end=4500, text="带中文与 emoji 😀"))
    write("ref_plain.ass", plain)
    write("ref.srt", rich)

    print("EMIT OK %d" % made[0])
    return made[0]


def main(argv):
    if len(argv) != 3 or argv[1] not in ("verify", "emit"):
        fail("usage: subs_pysubs2_check.py verify|emit <dir>")
    directory = Path(argv[2])
    if argv[1] == "verify":
        verify(directory)
    else:
        emit(directory)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
