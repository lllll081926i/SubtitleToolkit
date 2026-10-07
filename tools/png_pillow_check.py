#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""用 Pillow 做 PNG 的第三方交叉验证（image-core）。

Rust 侧只做自洽往返证明不了格式对：必须让另一个独立实现读我们的输出、
并且由它产出参考文件喂给我们的读取器。24 §QA / 25 §AT 的"外部参考"要求落地成这个脚本。

用法：
  python tools/png_pillow_check.py verify <dir>   # Pillow 打开 dir 下 ours_*.png，与 ours_*.rgba 比像素
  python tools/png_pillow_check.py emit <dir>     # Pillow 写 ref_*.png + ref_*.rgba 期望值
成功打印 VERIFY OK <n> / EMIT OK <n> 并退出 0；任何不一致打印差异并退出 1。
"""
from __future__ import annotations

import sys
from pathlib import Path

import PIL
from PIL import Image


def _fail(msg: str) -> None:
    print(f"PillowCheck FAIL: {msg}")
    sys.exit(1)


def verify(directory: Path) -> int:
    print(f"Pillow {PIL.__version__}")
    pngs = sorted(directory.glob("ours_*.png"))
    if not pngs:
        _fail(f"{directory} 下没有 ours_*.png，Rust 侧没写出任何东西")
    n = 0
    for png in pngs:
        sidecar = png.with_suffix(".rgba")
        if not sidecar.exists():
            _fail(f"{png.name} 缺少期望文件 {sidecar.name}")
        expected = sidecar.read_bytes()
        try:
            with Image.open(png) as im:
                im.load()
                got = im.convert("RGBA").tobytes()
                w, h = im.size
        except Exception as exc:  # noqa: BLE001 - 任何异常都算验证失败
            _fail(f"Pillow 打不开 {png.name}: {exc!r}")
        if len(got) != w * h * 4:
            _fail(f"{png.name} Pillow 长度与尺寸不符")
        if got != expected:
            if len(got) != len(expected):
                _fail(
                    f"{png.name} 长度不符：Pillow={len(got)} ours={len(expected)}（尺寸声明问题）"
                )
            first = next(i for i in range(len(got)) if got[i] != expected[i])
            _fail(
                f"{png.name} 像素不一致：size={w}x{h} 首个差异字节 {first}"
                f"（像素 {first // 4}）Pillow={got[first]} ours={expected[first]}"
            )
        n += 1
    print(f"VERIFY OK {n}")
    return n


def emit(directory: Path) -> int:
    directory.mkdir(parents=True, exist_ok=True)
    made = 0

    def write(name: str, im: Image.Image) -> None:
        nonlocal made
        path = directory / name
        im.save(path)
        # 期望值统一走 convert("RGBA")：与 image-core 的读回目标一致
        path.with_suffix(".rgba").write_bytes(im.convert("RGBA").tobytes())
        made += 1

    rgba = Image.new("RGBA", (24, 11), (0, 0, 0, 0))
    for y in range(11):
        for x in range(24):
            rgba.putpixel((x, y), (x * 10 % 256, y * 21 % 256, (x + y) * 7 % 256, 255 if (x % 3) else 0))
    write("ref_rgba.png", rgba)

    write("ref_rgb.png", rgba.convert("RGB"))
    write("ref_gray.png", rgba.convert("L"))
    la = rgba.convert("LA")
    write("ref_la.png", la)
    write("ref_bw.png", rgba.convert("1"))
    p = rgba.convert("P", palette=Image.Palette.WEB)
    write("ref_palette.png", p)
    p_t = p.copy()
    p_t.info["transparency"] = 0
    write("ref_palette_t.png", p_t)
    # 16 位不在这里做：Pillow 保存 I 模式已被标记弃用，16 位拒绝由 Rust 单测覆盖

    print(f"EMIT OK {made}")
    return made


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("verify", "emit"):
        _fail("usage: png_pillow_check.py verify|emit <dir>")
    directory = Path(argv[2])
    if not directory.is_dir():
        _fail(f"目录不存在：{directory}")
    if argv[1] == "verify":
        verify(directory)
    else:
        emit(directory)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
