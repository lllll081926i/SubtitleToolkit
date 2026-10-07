#!/usr/bin/env python3
"""为门阶生成一份"脚本自带 [Fonts]"的 ASS，并交代内嵌字节的内容哈希。

为什么要现场生成（而不是把 .ass 存进仓库）：附件正文是一整份字体的 base64，
那是 Microsoft 的二进制（21 §1 的许可口径），不能进版本库——与注入测试现场改名字体
同一个理由（ISSUES AK1）。

输出（stdout，供门阶 grep）：
    SHA256 <hex>     内嵌那批字节的哈希，门阶要求它出现在渲染台账里
    EMBED <path>     带 [Fonts] 的脚本
    PLAIN <path>     同一份脚本去掉 [Fonts] 的版本（身份必须不同，AK6）
"""

import base64
import hashlib
import os
import sys

ASS = """[Script Info]
ScriptType: v4.00+
PlayResX: 1280
PlayResY: 720
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: D,Microsoft YaHei UI,64,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,20,20,40,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:03.00,D,,0,0,0,,嵌入字体测试
"""


def main():
    out_dir = sys.argv[1]
    windir = os.environ.get("WINDIR", r"C:\Windows")
    src = os.path.join(windir, "Fonts", "marlett.ttf")
    if not os.path.isfile(src):
        print("MISSING-FONT " + src)
        return 2
    with open(src, "rb") as fh:
        data = fh.read()
    digest = hashlib.sha256(data).hexdigest()

    b64 = base64.b64encode(data).decode("ascii")
    # 形状照 libass 的 ass.c:process_fonts_line —— `fontname: X` 之后每一行都是这份字节的
    # base64，折行拼接，直到下一个 fontname: 或段末。这里就按真文件的写法生成（AK10）。
    lines = ["[Fonts]", "fontname: stk-embed.ttf"]
    for i in range(0, len(b64), 72):
        lines.append(b64[i : i + 72])
    # 附件段插在 [Events] 之前，与 libass/VSFilter 的常见写法一致
    embed = ASS.replace("[Events]", "\n".join(lines) + "\n\n[Events]", 1)

    embed_path = os.path.join(out_dir, "embed.ass")
    plain_path = os.path.join(out_dir, "plain.ass")
    with open(embed_path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(embed)
    with open(plain_path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(ASS)

    print("SHA256 " + digest)
    print("BYTES " + str(len(data)))
    print("EMBED " + embed_path)
    print("PLAIN " + plain_path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
