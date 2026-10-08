#!/usr/bin/env bash
# vcpkg 在墙内会卡在 "从 GitHub 下载 distfile 超时"（本轮实测：zlib 端口 3 次尝试全超时，
# error: Download timed out → error: building zlib:x64-windows failed）。
# 这个脚本做一件很窄的事：把 vcpkg 想要的那个 GitHub 文件用国内可达的 GitHub 加速前缀抓下来，
# 放进 vcpkg 的 downloads 目录，然后重跑 install —— 让 **vcpkg 自己按端口里的 SHA512 校验**。
# 校验不过 vcpkg 会报错，所以"镜像抓来的东西"不会悄悄变成信任链的一环。
set -u
VCPKG="${VCPKG:-../tools/vcpkg/vcpkg.exe}"
DL="${VCPKG_DOWNLOADS:-$(dirname "$VCPKG")/downloads}"
PORT="${VCPKG_PACKAGE:-libass:x64-windows}"
PORT_NAME="${PORT%%:*}"

# vcpkg 的安装账本是 installed/<triplet> 同级的 installed/vcpkg/status（Debian 风格 stanza，
# 成功时长这样：`Package: libass` ... `Status: install ok installed`）。
# 我第一版在这里猜了一个 share/<port>/vcpkg_status 路径，本机根本没有这个文件，
# 于是"成功判据"永远不成立（ISSUES Y2：猜的路径和猜的话术是同一类错误）。
STATUS_DB="$(dirname "$VCPKG")/installed/vcpkg/status"
mkdir -p "$DL"

# 只有该端口自己（不是它的依赖）记着 install ok installed 才算成功
installed_ok() {
    [ -f "$STATUS_DB" ] || return 1
    awk -v p="$PORT_NAME" '
        $1 == "Package:" && $2 == p { on = 1; next }
        $1 == "Package:" { on = 0 }
        on && $0 ~ /^Status:[ ]*install ok installed/ { found = 1 }
        END { exit found ? 0 : 1 }
    ' "$STATUS_DB"
}

for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
    echo "=== attempt $attempt: $VCPKG install $PORT"
    log=$("$VCPKG" install "$PORT" 2>&1)
    echo "$log" | tail -5
    if installed_ok; then
        echo "INSTALL SUCCEEDED（$PORT (vcpkg status: install ok installed)）"
        exit 0
    fi
    if echo "$log" | grep -q "Package libass was not handled"; then
        echo "PORT FAILED (not a download issue)"; exit 2
    fi
    if echo "$log" | grep -q "error: Download timed out\|won't retry download from"; then
        # 只认 vcpkg 明说"放弃重试"的那条 URL。以前我用"日志里最后一个 https URL"，
        # 结果抓到的是报错信息里的故障排查链接（learn.microsoft.com/...），
        # 白抓 12 轮还不报错 —— 取错目标比取不到目标更糟。
        url=$(echo "$log" | grep -oE "won't retry download from https://[^ ]+" | sed -E 's#.*from ##' | tail -1)
        if [ -z "$url" ]; then
            echo "没有可重试的下载 URL —— 这不是下载问题，是端口构建失败"
            echo "$log" | tail -40
            exit 3
        fi
        # vcpkg 把 github archive 存成 <owner>-<repo>-<ref>.tar.gz 之类；让 vcpkg 自己命名：
        # 用 gh-proxy 直接抓，成功后按 vcpkg 期望的文件名放进去
        leaf=$(basename "$url")
        case "$url" in
            */archive/*) leaf="$(echo "$url" | sed -E 's#https://github.com/##; s#/archive/##; s#/#-#g')" ;;
        esac
        for prefix in "https://gh-proxy.com/" "https://ghfast.top/" "https://mirror.ghproxy.com/"; do
            target="$DL/$leaf"
            echo "--- fetching $prefix$url -> $target"
            if curl -L --fail --connect-timeout 20 --max-time 900 -o "$target.part" "$prefix$url"; then
                mv "$target.part" "$target"
                # vcpkg 的 FILE_NAME 规则与 URL basename 不同；两种名字都放一份，命中率高
                alt="$DL/$(basename "$url")"
                [ "$alt" != "$target" ] && cp "$target" "$alt"
                sha512=$(sha512sum "$target" | cut -c1-16)
                echo "--- got $target (sha512 ${sha512}..., size $(stat -c%s "$target" 2>/dev/null))"
                break
            fi
        done
        continue
    fi
    if echo "$log" | grep -qiE "Successfully installed|already installed"; then
        # 话术说成功了，但 vcpkg_status 不是 OK：这是最需要人工看的一种不一致
        echo "日志显示成功，但 $STATUS_DB 里没有该端口的 install ok installed（半装状态）"
        echo "$log" | tail -40
        exit 5
    fi
    echo "unrecognized failure"; echo "$log" | tail -30; exit 4
done
echo "gave up after 12 attempts：$PORT 仍未通过 vcpkg_status 检查"
exit 1
