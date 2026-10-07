#!/usr/bin/env bash
# 生成 media-probe 的真实语料（16 §2、24 §QA：AT-PROBE-001 要真视频，不要手造 JSON）。
#
# 为什么要有这个脚本而不是只提交 mp4：验收证据必须能被别人重做。
# 每条命令都写在这里，并且最后把 ffprobe 的**原始输出**也存成 .ffprobe.json ——
# fields.rs 的单测拿那份真输出当 fixture，就不可能再按想象中的 schema 猜错（ISSUES AF1）。
#
# 用法：FFMPEG=/path/to/ffmpeg.exe bash tools/make_probe_fixtures.sh
# 两个二进制的来源与哈希记在 reference-audit/SOURCE_AUDIT_LOCK.json（不进仓库，属本机工具）。
set -eu
cd "$(dirname "$0")/.."

FFMPEG="${FFMPEG:-../tools/bin/ffmpeg.exe}"
FFPROBE="${FFPROBE:-../tools/bin/ffprobe.exe}"
[ -x "$FFMPEG" ] || { echo "找不到 ffmpeg：设 FFMPEG=... 指向它"; exit 2; }
[ -x "$FFPROBE" ] || { echo "找不到 ffprobe：设 FFPROBE=... 指向它"; exit 2; }

OUT=crates/media-probe/tests/fixtures
mkdir -p "$OUT"

# 1) 1080p25 SDR/bt709：帧率必须被读成精确的 25/1
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i testsrc2=size=1920x1080:rate=25:duration=2 \
  -c:v libx264 -preset ultrafast -crf 45 -pix_fmt yuv420p \
  -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
  "$OUT/fhd25.mp4"

# 2) 2160p23.976 + BT.2020/PQ：16 §3 规则 1 的 HDR preset 靠它
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i testsrc2=size=3840x2160:rate=24000/1001:duration=1 \
  -c:v libx264 -preset ultrafast -crf 51 -pix_fmt yuv420p \
  -colorspace bt2020nc -color_primaries bt2020 -color_trc smpte2084 \
  "$OUT/uhd23976-hdr.mp4"

# 3) 4:3 30fps：非 16:9 不许被建议成画布
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i testsrc2=size=640x480:rate=30:duration=1 \
  -c:v mpeg4 "$OUT/sd43-30.mp4"

# 4) 抓真工具的原始 JSON 当解析 fixture（相对路径调用，别把机器路径写进证据里）
for f in fhd25 uhd23976-hdr; do
  "$FFPROBE" -v quiet -print_format json -show_format -show_streams \
    "$OUT/$f.mp4" > "$OUT/$f.ffprobe.json"
done

echo "生成的文件："
ls -la "$OUT"
echo
echo "自检：色彩字段必须能对上"
for f in fhd25 uhd23976-hdr; do
  printf '  %-16s ' "$f"
  grep -o '"color_[a-z_]*": "[^"]*"' "$OUT/$f.ffprobe.json" | tr '\n' ' '
  echo
done
