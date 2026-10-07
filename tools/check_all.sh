#!/usr/bin/env bash
# CI 等价的验证门阶（28 §7、24 §QA、39 §清单）。
#
# 为什么是这个脚本而不是 PowerShell 版：第一版用 PowerShell 写，跑起来"全绿"却是假的
# （$results 在函数作用域里被丢掉，摘要一行都没有）。门阶自己必须先能证明它记录了结果，
# 所以这里改成单一 bash 实现，每个门阶用退出码表态：0=PASS、1=FAIL、2=SKIP。
# tools/check_all.ps1 只转发到本脚本，避免两份实现漂移。
#
#   bash tools/check_all.sh            # 允许 SKIP
#   bash tools/check_all.sh --strict   # 缺依赖的 SKIP 也算 FAIL（发布机/CI 用）
#   RELEASE=1 bash tools/check_all.sh  # 额外跑 release 构建（慢）
set -u
cd "$(dirname "$0")/.." || exit 1

STRICT=0
[ "${1:-}" = "--strict" ] && STRICT=1
LOGDIR=target/gate
mkdir -p "$LOGDIR"
SUMMARY="$LOGDIR/summary.txt"
# 摘要必须是"这一次运行"的结果。第一版只清 last-run.txt 不清 summary.txt，
# 于是它把十几次历史运行一路追加下去，读的人无法判断哪一行属于本次门阶
# （ISSUES Z5：证据文件混进历史，等于证据没有署名时间）。
: > "$SUMMARY"
: > "$LOGDIR/last-run.txt"
FAILED=0
SKIPPED=0

say() { printf '%s\n' "$*" >> "$LOGDIR/last-run.txt"; printf '%s\n' "$*"; }

start() { # name ref requirement
    say ""
    say "=== $1  [$2]"
    say "    $3"
    GATE_NAME="$1"
    CUR=$SECONDS
}

report() { # rc note
    local rc="$1" note="${2:-}" dur=$((SECONDS - CUR))
    case "$rc" in
        0) say "    PASS (${dur}s)"; printf '%-30s PASS  %ss\n' "$GATE_NAME" "$dur" >> "$SUMMARY" ;;
        2)
            if [ "$STRICT" = 1 ]; then
                say "    SKIP -> FAIL (--strict)：$note"
                printf '%-30s FAIL  %s\n' "$GATE_NAME" "$note" >> "$SUMMARY"
                FAILED=$((FAILED + 1))
            else
                say "    SKIP：$note"
                printf '%-30s SKIP  %s\n' "$GATE_NAME" "$note" >> "$SUMMARY"
                SKIPPED=$((SKIPPED + 1))
            fi
            ;;
        *)
            say "    FAIL (${dur}s)：$note"
            printf '%-30s FAIL  %s\n' "$GATE_NAME" "${note:-非零退出}" >> "$SUMMARY"
            FAILED=$((FAILED + 1))
            ;;
    esac
}

# ---------------------------------------------------------------- 0 门阶脚本自检
g_self() {
    bash -n tools/check_all.sh && bash -n tools/vcpkg_mirror_retry.sh
}
start gate-selftest "28 §7" "门阶自己必须先语法可执行（本文件已两次被手工编辑改坏过语法）"
g_self; report $? "shell 语法错误"

# ---------------------------------------------------------------- 1 格式
g_fmt() { cargo fmt --all --check; }
start format "28 §7" "cargo fmt --all --check"
g_fmt; report $? "格式不干净（cargo fmt --all）"

# ---------------------------------------------------------------- 2 lint
g_clippy() { cargo clippy --workspace --all-targets -- -D warnings; }
start clippy "28 §7、23" "cargo clippy --workspace --all-targets -- -D warnings"
g_clippy; report $? "clippy 有 error/warning"

# ---------------------------------------------------------------- 3 测试
g_tests() {
    if command -v cargo-nextest >/dev/null 2>&1; then
        # nextest 的输出同样要落到 tests.log：第一版只有 cargo test 分支做了重定向，
        # 于是 "见 target/gate/tests.log" 指的是上一次运行的残留文件（ISSUES Z7）
        cargo nextest run --workspace > "$LOGDIR/tests.log" 2>&1 || return 1
        # LEAK 不算测试失败，但它是"进程里留下了活物"的信号，必须显式出现在摘要里。
        # 本机现在有 1 例已知泄漏：media-probe 的超时测试在 nextest 下无法附着 Job Object
        # （宿主已把我们放进外层 Job），未被约束的孙进程继承了 nextest 的 stdout 管道句柄，
        # 于是管道迟迟不 EOF。彻底修需要 CreateProcess 的 PROC_THREAD_ATTRIBUTE_HANDLE_LIST
        # 来限制句柄继承 —— 记在 ISSUES Z6，未解决；门阶不把它藏起来，但也不让它冒充失败。
        grep -E "^[[:space:]]*(LEAK|Summary)" "$LOGDIR/tests.log" | sed 's/^/    /' || true
    else
        say "    (没有 cargo-nextest，用 cargo test)"
        cargo test --workspace > "$LOGDIR/tests.log" 2>&1
    fi
}
start tests "28 §7、24 §QA" "全部单元 + 集成测试（nextest 可用则用 nextest）"
g_tests; rc=$?
report $rc "见 $LOGDIR/tests.log（$(grep -c 'FAILED\|panicked' "$LOGDIR/tests.log" 2>/dev/null || echo 0) 处失败线索）"
grep -oE "test result: ok. [0-9]+ passed" "$LOGDIR/tests.log" 2>/dev/null | awk '{s+=$4} END {if (s>0) print "    合计通过测试：" s}'

# ---------------------------------------------------------------- 4 外部实现交叉验证
g_pillow() {
    command -v python >/dev/null 2>&1 || return 2
    python -c "import PIL" > /dev/null 2>&1 || return 2
    STK_REQUIRE_PILLOW=1 cargo test -p image-core --test pillow_cross -- --nocapture > "$LOGDIR/pillow.log" 2>&1
}
start png-cross-check "24 §QA、25 AT-BDN-001" "Pillow 读我们写的 PNG，我们读 Pillow 写的 PNG（不接受自证）"
g_pillow; rc=$?
[ $rc -eq 2 ] && report 2 "本机没有 python + Pillow" || {
    report $rc "见 $LOGDIR/pillow.log"
    grep -E "VERIFY OK|EMIT OK" "$LOGDIR/pillow.log" 2>/dev/null | sed 's/^/    /'
}

# ---------------------------------------------------------------- 5 CLI 端到端
g_cli() {
    cargo build -p cli > /dev/null 2>&1 || return 1
    local exe=target/debug/subtitletool-cli.exe
    [ -f "$exe" ] || return 2
    local tmp; tmp=$(mktemp -d)
    printf '1\n00:00:01,000 --> 00:00:03,000\nhello\n\n2\n00:00:04,000 --> 00:00:06,000\nworld\n' > "$tmp/a.srt"
    "$exe" nosuchcommand > /dev/null 2>&1
    [ $? -eq 2 ] || { say "    用法错误没返回退出码 2"; rm -rf "$tmp"; return 1; }
    local line; line=$("$exe" probe "$tmp/a.srt" --json 2>&1 | tr -d '\n')
    printf '%s' "$line" | grep -q '"schema"' || { say "    信封缺 schema：$line"; rm -rf "$tmp"; return 1; }
    printf '%s' "$line" | grep -q '"command":"probe"' || { say "    信封缺 command：$line"; rm -rf "$tmp"; return 1; }
    [ "$(printf '%s\n' "$line" | wc -l)" -le 1 ] || { say "    JSON 不是单行"; rm -rf "$tmp"; return 1; }
    "$exe" convert "$tmp/a.srt" --to ass --out "$tmp/a.ass" --dry-run --json > /dev/null 2>&1
    [ ! -e "$tmp/a.ass" ] || { say "    --dry-run 写出了文件"; rm -rf "$tmp"; return 1; }
    "$exe" convert "$tmp/a.srt" --to ass --out "$tmp/a.ass" --json > /dev/null 2>&1 || { say "    convert 失败"; rm -rf "$tmp"; return 1; }
    [ -e "$tmp/a.ass" ] || { say "    convert 没产出文件"; rm -rf "$tmp"; return 1; }
    "$exe" time "$tmp/a.ass" --offset-ms -1000 --out "$tmp/b.ass" --json > /dev/null 2>&1 || { say "    time --offset-ms 失败"; rm -rf "$tmp"; return 1; }
    # 15 §1/§8：batch 必须一次产出多个 profile + 可读回的报告 + 恢复状态
    printf '{"id":"gate","roots":["."],"include":["*.ass"],"profiles":[{"name":"sub","format":"srt"},{"name":"txt","format":"ass"}],"output":{"dir":"out","template":"{stem}.{profile}.{ext}"},"video_association":false}' > "$tmp/job.json"
    local bref; bref=$("$exe" batch --spec "$tmp/job.json" --out "$tmp/report.json" --json 2>&1 | tr -d '\n')
    printf '%s' "$bref" | grep -q '"outcome":"success"' || { say "    batch 没判成功：$bref"; rm -rf "$tmp"; return 1; }
    [ -f "$tmp/out/a.sub.srt" ] && [ -f "$tmp/out/a.txt.ass" ] || { say "    batch 的多输出没落盘"; rm -rf "$tmp"; return 1; }
    [ -f "$tmp/report.json" ] || { say "    batch 报告没写"; rm -rf "$tmp"; return 1; }
    [ -f "$tmp/out/.jobstate.json" ] || { say "    batch 没写恢复状态（15 §7）"; rm -rf "$tmp"; return 1; }
    # 再跑一次必须复用（哈希与分量都没变），否则会静默重做已经验证过的产物
    local bref2; bref2=$("$exe" batch --spec "$tmp/job.json" --out "$tmp/report2.json" --json 2>&1 | tr -d '\n')
    printf '%s' "$bref2" | grep -q '"from-cache"' || { say "    第二次 batch 没复用已验证产物：$bref2"; rm -rf "$tmp"; return 1; }
    # doctor 的未实现清单必须由解析器推导：列出来的命令真的回 0060
    "$exe" doctor --json 2>&1 | tr -d '\n' | grep -q '"not_built":\[' || { say "    doctor 没有 not_built 清单"; rm -rf "$tmp"; return 1; }
    for done_cmd in "pgs encode" "batch" "render"; do
        if "$exe" doctor --json 2>&1 | tr -d '\n' | grep -q "\"$done_cmd\""; then
            say "    doctor 仍把已实现的 $done_cmd 列为未实现（清单必须是推出来的）"; rm -rf "$tmp"; return 1
        fi
    done
    # 占位命令必须"明确失败"而不是假成功。font subset/mkv 是现在真正没接线的两条
    local ph; ph=$("$exe" mkv extract "$tmp/a.ass" --json 2>&1 | tr -d '\n')
    printf '%s' "$ph" | grep -q 'STK-INP-0060' || { say "    未实现命令没报 STK-INP-0060：$ph"; rm -rf "$tmp"; return 1; }
    if "$exe" mkv extract "$tmp/a.ass" --json > /dev/null 2>&1; then
        say "    未实现命令退出 0（假成功）"; rm -rf "$tmp"; return 1
    fi
    # 已实现的 `pgs encode` / `render` 在没有渲染器时同样不能假成功：退出码 1、错误码点名缺什么、不落脚半成品
    rm -f "$tmp/a.sup"
    if env -u STK_LIBASS_DLL "$exe" pgs encode "$tmp/a.ass" --out "$tmp/a.sup" --json > /dev/null 2>&1; then
        say "    缺 libass 时 pgs encode 竟然退出 0"; rm -rf "$tmp"; return 1
    fi
    local noenv; noenv=$(env -u STK_LIBASS_DLL "$exe" pgs encode "$tmp/a.ass" --out "$tmp/a.sup" --json 2>&1 | tr -d '\n')
    printf '%s' "$noenv" | grep -q 'STK-INP-0141' || { say "    缺渲染器没报 STK-INP-0141：$noenv"; rm -rf "$tmp"; return 1; }
    [ ! -e "$tmp/a.sup" ] || { say "    失败的 pgs encode 留下了半成品（AT-OUT-001）"; rm -rf "$tmp"; return 1; }
    if env -u STK_LIBASS_DLL "$exe" render "$tmp/a.ass" --out "$tmp/bdn/a.xml" --json > /dev/null 2>&1; then
        say "    缺 libass 时 render 竟然退出 0"; rm -rf "$tmp"; return 1
    fi
    [ ! -e "$tmp/bdn" ] || { say "    失败的 render 留下了半成品目录（AT-OUT-001）"; rm -rf "$tmp"; return 1; }
    # 非 16:9 画布是用法错误（2 而不是 1）：09 §1 的画布 profile 在参数层就该拦住
    env -u STK_LIBASS_DLL "$exe" render "$tmp/a.ass" --canvas 1024x768 --json > /dev/null 2>&1
    [ $? -eq 2 ] || { say "    render 的非 16:9 画布没返回退出码 2"; rm -rf "$tmp"; return 1; }
    "$exe" doctor --json > /dev/null 2>&1 || { say "    doctor 失败"; rm -rf "$tmp"; return 1; }
    rm -rf "$tmp"
}
start cli-smoke "20、22、25" "真二进制：信封字段 / 单行 JSON / 退出码 0-1-2 / --dry-run 不写文件 / 未实现命令不假成功"
g_cli; report $? "CLI 端到端契约不成立"

# ---------------------------------------------------------------- 6 Strict 徽章不能自封
g_strict() {
    cargo test -p cli --test cli pgs_validate_reports_structure_for_streams_we_encoded \
        > "$LOGDIR/strict.log" 2>&1
}
start strict-badge "13 §6、39" "按 Strict 校验必须被 PGS.DEC.01 / PGS.DEV.01 拦下，且 findings 仍完整交出去"
g_strict; report $? "见 $LOGDIR/strict.log"

# ---------------------------------------------------------------- 7 模糊冒烟（28 §7 fuzz smoke）
g_fuzz() {
    cargo test -p cli --test fuzz_smoke -- --nocapture > "$LOGDIR/fuzz.log" 2>&1
}
start fuzz-smoke "28 §7、23 §健壮性" "变异字节喂给 4 个解析器 + 真实 CLI 入口：不许 panic、失败不许留半成品"
g_fuzz; rc=$?
report $rc "见 $LOGDIR/fuzz.log"
grep -E "convert 变异结果|test result" "$LOGDIR/fuzz.log" 2>/dev/null | sed 's/^/    /'

# ---------------------------------------------------------------- 8 依赖与许可
g_deny() {
    command -v cargo-deny >/dev/null 2>&1 || return 2
    [ -f deny.toml ] || return 2
    cargo deny check licenses bans sources > "$LOGDIR/deny.log" 2>&1
}
start dependency-audit "21 §11、28 §1" "cargo deny check licenses/bans/sources（离线可做）"
g_deny; rc=$?
[ $rc -eq 2 ] && report 2 "cargo-deny 或 deny.toml 缺失" || report $rc "见 $LOGDIR/deny.log"

# ---------------------------------------------------------------- 9a RustSec 通告（要联网）
g_adv() {
    command -v cargo-deny >/dev/null 2>&1 || return 2
    # 仓库内临时的 insteadOf 镜像配置：不修改用户全局 git 配置，也不改变结论来源的语义
    mkdir -p "$LOGDIR/gitcfg"
    printf '[url "https://gh-proxy.com/https://github.com/"]
	insteadOf = https://github.com/
' > "$LOGDIR/gitcfg/config"
    GIT_CONFIG_GLOBAL="$PWD/$LOGDIR/gitcfg/config" cargo deny check advisories > "$LOGDIR/advisories.log" 2>&1
    if grep -qi "failed to fetch advisory database\|Could not connect" "$LOGDIR/advisories.log"; then
        return 2
    fi
}
start advisory-audit "28 §1" "cargo deny check advisories（需要能拉到 RustSec advisory-db）"
g_adv; rc=$?
[ $rc -eq 2 ] && report 2 "拉不到 RustSec advisory-db（GitHub 连接抖动，本机曾成功过）" || report $rc "见 $LOGDIR/advisories.log"

# ---------------------------------------------------------------- 8b 字幕语义交叉验证
g_subs() {
    command -v python >/dev/null 2>&1 || return 2
    python -c "import pysubs2" > /dev/null 2>&1 || return 2
    STK_REQUIRE_PYSUBS2=1 cargo test -p codec-ass --test pysubs2_cross -- --nocapture > "$LOGDIR/subs.log" 2>&1
}
start subs-cross-check "24 §QA、25 AT-ASS-001" "pysubs2 读我们写的 ASS/SRT，我们读 pysubs2 写的（语义级，不是字节级）"
g_subs; rc=$?
[ $rc -eq 2 ] && report 2 "本机没有 python + pysubs2" || { report $rc "见 $LOGDIR/subs.log"; grep -E "VERIFY OK|EMIT OK" "$LOGDIR/subs.log" 2>/dev/null | sed 's/^/    /'; }

# ---------------------------------------------------------------- 9b 内存上限（AT-SEC-001）
g_sec() { cargo test -p codec-pgs --test security > "$LOGDIR/security.log" 2>&1; }
start security-limits "25 AT-SEC-001、23 §健壮性" "恶意 length/RLE 不崩溃、不超额分配（用计数分配器实测字节数）"
g_sec; report $? "见 $LOGDIR/security.log"

# ---------------------------------------------------------------- 9e 真实 libass 渲染（native 依赖）
# 这一条存在的原因：ASS_Image 的 alpha 字节是"透明度"而不是直通 alpha，
# 这种方向性错误只有真渲染能暴露——纯 Rust 的单测两边都由同一个人写，自洽地错过一次
# （ISSUES Z1）。渲染结果还必须一路走到 PGS 段流，量化预算那条坑也是这样撞出来的（Z3）。
g_render() {
    local dll="${STK_LIBASS_DLL:-}"
    [ -n "$dll" ] || dll="../tools/vcpkg/installed/x64-windows/bin/ass-9.dll"
    # 必须换成绝对路径再交给测试：cargo 跑测试二进制的 cwd 是 crate 目录，
    # 相对路径在这里既过不了 `[ -f ]`，也过不了 LoadLibraryExW（ISSUES Z4）
    dll=$(readlink -f "$dll" 2>/dev/null || printf '%s' "$dll")
    [ -f "$dll" ] || return 2
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" cargo test -p render-libass --test render_real \
        > "$LOGDIR/render.log" 2>&1 || return 1
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" cargo test -p pgs-author --test pipeline \
        > "$LOGDIR/render-pipeline.log" 2>&1 || return 1
    # 渲染结果一路走到 CLI 交付物：BDN 目录、dry-run、覆盖策略、目录型产物的复用判据
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" cargo test -p cli --test render \
        > "$LOGDIR/cli-render.log" 2>&1 || return 1
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" cargo test -p cli --test batch bdn \
        > "$LOGDIR/cli-bdn.log" 2>&1 || return 1
    # 14 §ASS→SUP 那句"共享 render/cache"：只渲一次是可测的数字，不是一句承诺
    # （filter "share" 同时命中正向的 shares_... 与反向控制的 does_not_share_...）
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" cargo test -p cli --test batch share \
        > "$LOGDIR/cli-share.log" 2>&1 || return 1
    # 17 §7：注入这条也要在门阶的**真 CLI 输出**上看得见，不能只在测试里绿。
    # 三件事分开验：开了要说开并交出内容哈希；没开要明确说没开且一个字节都没交；
    # 脚本自带 [Fonts] 时那批字节要既进台账又进渲染身份。
    # 核对方式从 grep 换成了语义检查器（ISSUES AK7）：serde_json 把对象键按字母序排，
    # 给报告加一个 embedded 字段就把 enabled 挤到了第二位，于是"功能正常、门阶变红"。
    command -v python >/dev/null 2>&1 \
        || { say "    门阶需要 python 来核对注入报告（28 §1 工具链）"; return 1; }
    local itmp; itmp="$(readlink -f "$LOGDIR")/inject-cli"
    rm -rf "$itmp"; mkdir -p "$itmp"
    # 每次渲染各占一个输出目录：BDN 是"XML + 同目录的 PNG"，四次渲染共用一个目录时，
    # 后一次的 PNG 会被覆盖策略改名成 0000(1).png，XML 声明的尺寸就对不上实际文件了
    # （STK-BDN-0073）。这是门阶自己的夹具设计错，不是产品缺陷，但必须先排除掉再谈结论。
    mkdir -p "$itmp/inject" "$itmp/noinject" "$itmp/embed" "$itmp/plain"
    cp crates/codec-ass/tests/fixtures/full22.ass "$itmp/a.ass"
    STK_LIBASS_DLL="$dll" target/debug/subtitletool-cli.exe render "$itmp/a.ass" \
        --inject-fonts --out "$itmp/inject/a.xml" --json > "$LOGDIR/inject-cli.log" 2>&1 \
        || { say "    --inject-fonts 的渲染跑不起来：见 $LOGDIR/inject-cli.log"; return 1; }
    python tools/check_injection_report.py "$LOGDIR/inject-cli.log" true 1 \
        > "$LOGDIR/inject-check.log" 2>&1 \
        || { say "    注入台账的语义核对没过：见 $LOGDIR/inject-check.log"; return 1; }
    STK_LIBASS_DLL="$dll" target/debug/subtitletool-cli.exe render "$itmp/a.ass" \
        --out "$itmp/noinject/b.xml" --json > "$LOGDIR/noinject-cli.log" 2>&1 || return 1
    python tools/check_injection_report.py "$LOGDIR/noinject-cli.log" false 0 \
        > "$LOGDIR/noinject-check.log" 2>&1 \
        || { say "    没开注入时报告也要讲清楚：见 $LOGDIR/noinject-check.log"; return 1; }

    # 17 §7 第二句 + 17 §6「embed ASS [Fonts]」的读侧：字体文件不入库，样本现场生成
    # （21 §1 许可口径，同 AK1 的理由）。字体字节 base64 进 [Fonts] 段，取 marlett。
    python tools/make_embedded_fixture.py "$itmp" > "$LOGDIR/embed-fixture.log" 2>&1 \
        || { say "    造不出带 [Fonts] 的样本：见 $LOGDIR/embed-fixture.log"; return 1; }
    local esha edim pdim
    esha=$(sed -n 's/^SHA256 //p' "$LOGDIR/embed-fixture.log" | head -1)
    # 空哈希会把下面的 grep -F 变成"匹配一切"，所以先验长度（AI9 的那类自欺）
    [ "${#esha}" -eq 64 ] \
        || { say "    样本生成器没交出内容哈希（拿到的是 '$esha'），后续核对无从谈起"; return 1; }
    STK_LIBASS_DLL="$dll" target/debug/subtitletool-cli.exe render "$itmp/embed.ass" \
        --inject-fonts --out "$itmp/embed/e.xml" --json > "$LOGDIR/embed-cli.log" 2>&1 \
        || { say "    带 [Fonts] 的脚本渲不动：见 $LOGDIR/embed-cli.log"; return 1; }
    python tools/check_injection_report.py "$LOGDIR/embed-cli.log" true 1 1 \
        > "$LOGDIR/embed-check.log" 2>&1 \
        || { say "    内嵌字节的台账核对没过：见 $LOGDIR/embed-check.log"; return 1; }
    grep -Fq "$esha" "$LOGDIR/embed-cli.log" \
        || { say "    台账里没有内嵌那批字节的哈希 $esha：[Fonts] 的字节没被交出去"; return 1; }
    # 同一份脚本去掉 [Fonts] 后身份必须变（AK6：内嵌字体不进摘要的话，缓存会串脚本）
    STK_LIBASS_DLL="$dll" target/debug/subtitletool-cli.exe render "$itmp/plain.ass" \
        --inject-fonts --out "$itmp/plain/p.xml" --json > "$LOGDIR/plain-cli.log" 2>&1 \
        || { say "    去掉 [Fonts] 的同一份脚本渲不动：见 $LOGDIR/plain-cli.log"; return 1; }
    python tools/check_injection_report.py "$LOGDIR/plain-cli.log" true 1 0 \
        > "$LOGDIR/plain-check.log" 2>&1 \
        || { say "    对照样本的核对没过：见 $LOGDIR/plain-check.log"; return 1; }
    edim=$(sed -n 's/.*digest=\([0-9a-f]*\).*/\1/p' "$LOGDIR/embed-check.log" | head -1)
    pdim=$(sed -n 's/.*digest=\([0-9a-f]*\).*/\1/p' "$LOGDIR/plain-check.log" | head -1)
    [ -n "$edim" ] && [ -n "$pdim" ] || { say "    没取到两份身份摘要，比较不成立"; return 1; }
    [ "$edim" != "$pdim" ] \
        || { say "    带 [Fonts] 与不带的两份脚本算出同一个身份摘要 $edim（AK6 的钉失效）"; return 1; }
    say "    注入核对：$(cat "$LOGDIR/inject-check.log")"
    say "    内嵌核对：$(cat "$LOGDIR/embed-check.log") 内嵌哈希 $esha 在台账里，身份摘要随 [Fonts] 改变"
    rm -rf "$itmp"
}
start render-real "12 §1A、13 §4/§5、14 §ASS→BDN" "真 libass 渲染出像素，并把像素一路 author 成可读回的 PGS 段流与 BDN 目录（不等于 AT-RENDER-001 的 host 隔离）"
g_render; rc=$?
if [ $rc -eq 2 ]; then
    report 2 "本机没有 ass-9.dll（设 STK_LIBASS_DLL 指到它，或 bash tools/vcpkg_mirror_retry.sh 构建）"
else
    report $rc "见 $LOGDIR/render.log 与 $LOGDIR/render-pipeline.log"
    grep -hE "libass library version|images=|test result: ok" \
        "$LOGDIR/render.log" "$LOGDIR/render-pipeline.log" 2>/dev/null | sed 's/^/    /'
fi

# ---------------------------------------------------------------- 9f renderer-host 进程隔离
# 12 §2 的整条承诺："第三方 DLL 永远不进入主进程"。这里考的不是能不能出图，
# 而是位数不符 / 崩溃 / 挂住这三种失败会不会只打死 host。
# 它同时是 28 §2 要的产物（renderer-host-x64）第一次被真的构建并被真的调用。
g_host() {
    local dll="${STK_LIBASS_DLL:-}"
    [ -n "$dll" ] || dll="../tools/vcpkg/installed/x64-windows/bin/ass-9.dll"
    dll=$(readlink -f "$dll" 2>/dev/null || printf '%s' "$dll")
    [ -f "$dll" ] || return 2
    cargo build -p renderer-host --bin subtitletool-render-host \
        > "$LOGDIR/host-build.log" 2>&1 || return 1
    local hx
    hx=$(readlink -f target/debug/subtitletool-render-host.exe 2>/dev/null)
    [ -n "$hx" ] && [ -f "$hx" ] || return 2
    STK_REQUIRE_LIBASS=1 STK_LIBASS_DLL="$dll" STK_RENDERER_HOST="$hx" \
        cargo test -p renderer-host > "$LOGDIR/host-test.log" 2>&1 || return 1
    # doctor 必须把这项能力如实报出来：它是用户问"能不能隔离渲染"时的答案来源
    local rep
    rep=$(STK_LIBASS_DLL="$dll" STK_RENDERER_HOST="$hx" \
        target/debug/subtitletool-cli.exe doctor --json 2>/dev/null | tr -d '\n')
    printf '%s' "$rep" | grep -q '"render_host"' || { say "    doctor 没有 render_host 段"; return 1; }
    printf '%s' "$rep" | grep -q '"available":true' || { say "    doctor 说 host 不可用：$rep"; return 1; }
    printf '%s' "$rep" | grep -q '"tree_contained"' || { say "    doctor 没交代孙进程约束状态"; return 1; }
    printf '%s' "$rep" | grep -q '"provider_arch"' || { say "    握手没报位数"; return 1; }
}
start renderer-host "12 §2/§3、23 §外部进程、28 §2" "位数不符的 DLL 在进程边界就被拒；崩溃/挂住只终止 host 并带回退出码与日志；doctor 如实报告"
g_host; rc=$?
if [ $rc -eq 2 ]; then
    report 2 "本机没有 ass-9.dll 或 host 没构建出来（设 STK_LIBASS_DLL）"
else
    report $rc "见 $LOGDIR/host-test.log 与 $LOGDIR/host-build.log"
    grep -hE "test result: ok" "$LOGDIR/host-test.log" 2>/dev/null | sed 's/^/    /'
fi

# ---------------------------------------------------------------- 9g 真 ffprobe 探测（16 §2/§3）
# 这条存在的直接理由：色彩字段此前按想象中的嵌套 schema 解析，单测全绿而真二进制读不出东西
# （ISSUES AF1）。所以 fixture 必须由真工具产生、并由真工具来验。
g_probe() {
    local fp="${STK_FFPROBE:-}"
    [ -n "$fp" ] || fp="../tools/bin/ffprobe.exe"
    fp=$(readlink -f "$fp" 2>/dev/null || printf '%s' "$fp")
    [ -f "$fp" ] || return 2
    STK_REQUIRE_FFPROBE=1 STK_FFPROBE="$fp" cargo test -p media-probe --test real_probe \
        > "$LOGDIR/probe.log" 2>&1 || return 1
    # 用户能看到的那条路径也要带上建议与色彩证据，否则 crate 对了、CLI 还是瞎的
    local out
    out=$(STK_FFPROBE="$fp" target/debug/subtitletool-cli.exe probe \
        crates/media-probe/tests/fixtures/uhd23976-hdr.mp4 --json 2>&1 | tr -d '\n')
    printf '%s' "$out" | grep -q '"is_hdr":true' || { say "    CLI probe 没判出 HDR：$out"; return 1; }
    printf '%s' "$out" | grep -q '"hdr-preset"' || { say "    CLI probe 没给 HDR preset：$out"; return 1; }
    printf '%s' "$out" | grep -q '"color_transfer": "smpte2084"' \
        || printf '%s' "$out" | grep -q '"transfer":"smpte2084"' \
        || { say "    CLI probe 没交出色彩证据：$out"; return 1; }
    printf '%s' "$out" | grep -q '"recommends_speed_conversion":false' \
        || { say "    探测输出出现了自动降速承诺：$out"; return 1; }
}
start probe-real "16 §2/§3、24 §QA、25 AT-PROBE-001" "真 ffprobe + 真视频：尺寸/帧率/色彩/建议都要从工具输出里读出来，且永不建议降速"
g_probe; rc=$?
if [ $rc -eq 2 ]; then
    report 2 "本机没有 ffprobe（设 STK_FFPROBE，或 bash tools/make_probe_fixtures.sh 之前先装二进制）"
else
    report $rc "见 $LOGDIR/probe.log"
    grep -hE "test result: ok" "$LOGDIR/probe.log" 2>/dev/null | sed 's/^/    /'
fi

# ---------------------------------------------------------------- 9h 真字体库扫描（17 §2-§4）
# 这条存在的理由：字体这件事用合成 fixture 测不出真事实。本机 Arial 画不出中文、
# full22.ass 要的 "Microsoft YaHei UI" 到底装没装 —— 只有对着真字体目录跑才看得见（ISSUES AG3）。
g_font() {
    local dir="${STK_FONTS_DIR:-C:/Windows/Fonts}"
    [ -d "$dir" ] || return 2
    ls "$dir"/*.tt[fc] >/dev/null 2>&1 || return 2
    STK_REQUIRE_FONTS=1 STK_FONTS_DIR="$dir" cargo test -p font-service \
        > "$LOGDIR/font.log" 2>&1 || return 1
    local rep cpath
    rep="$(readlink -f "$LOGDIR")/font-report.json"
    rm -f "$LOGDIR/font-report.json"
    # AI5：字体库磁盘缓存指到门阶自己的路径，免得一次梯子读到另一次梯子的残留
    cpath="$(readlink -f "$LOGDIR")/font-cache.json"
    rm -f "$cpath"
    STK_FONTCACHE="$cpath" target/debug/subtitletool-cli.exe font scan \
        crates/codec-ass/tests/fixtures/full22.ass \
        --fonts-dir "$dir" --out "$rep" --json > "$LOGDIR/font-cli.log" 2>&1 || return 1
    grep -q '"command":"font scan"' "$LOGDIR/font-cli.log" \
        || { say "    信封没认领命令：见 $LOGDIR/font-cli.log"; return 1; }
    grep -q '"state":"written"' "$LOGDIR/font-cli.log" \
        || { say "    --out 的报告没落盘"; return 1; }
    # 报告是 to_vec_pretty 写的，冒号后面有空格：`"faces": 196`。
    # 上一版按紧凑 JSON 的写法 grep（`"faces":[1-9]`），只在信封那一份紧凑输出里能命中，
    # 于是门阶拿着"落盘的报告读不到 face"这句假结论 FAIL（ISSUES AG13）。
    grep -Eq '"faces": *[1-9]' "$rep" || { say "    真字体库读不到 face：$rep"; return 1; }
    grep -q 'DirectWrite' "$rep" || { say "    报告没交代 17 §2 的权威边界"; return 1; }
    # 17 §7：报告必须给出渲染身份（缓存 key 里那一维），且每条请求都在里面占一格
    grep -q '"render_identity"' "$rep" || { say "    报告没有 render_identity，缓存身份没接上"; return 1; }
    # 六档评级必须都在报告的词汇表里（Missing Glyph 曾经被吞成 Exact，见 ISSUES AG4）
    grep -q 'missing_glyph' "$rep" || { say "    报告里没有 17 §4 的 missing_glyph 档"; return 1; }
    # 报告与信封必须讲同一个故事（同一份数据，两种形状）
    grep -Eq '"command":"font scan".*"ok":true' "$LOGDIR/font-cli.log" \
        || { say "    信封没有 ok:true 的 font scan 结果"; return 1; }
    # 17 §2 的"字体注册"：注册表必须真的被读到，且每条登记都有交代
    grep -Eq '"resolved": *[1-9]' "$rep" \
        || { say "    注册表一条都没解析到，枚举没生效：$rep"; return 1; }
    # 17 §6 第二档：整份复制 + manifest。用一份临时字体目录跑，测的是交付路径本身。
    # 复制的必须是 full22.ass 真的请求的那几家：随便挑一个 .ttf 会得到"没有字体可交付"
    # （那是产品的正确回答，但不是这条门阶想验的东西）。
    # AI5：这一跑是从零建库（缓存文件刚被删掉），报告必须这么说
    grep -q '"cache": "scanned"' "$rep"         || { say "    第一跑该是全量扫描，报告却没这么说：$rep"; return 1; }
    # 同一件事再做一次：必须吃到缓存，而且**渲染身份一个字都不能变**。
    # 这两条合起来才是缓存存在的意义 —— 只改代价，不改答案。
    # （撒谎的缓存会怎样，由 font-service/tests/font_cache.rs 与 cli 的 fontenv 测试锁。）
    local rep2
    rep2="$(readlink -f "$LOGDIR")/font-report-hit.json"
    rm -f "$rep2"
    STK_FONTCACHE="$cpath" target/debug/subtitletool-cli.exe font scan         crates/codec-ass/tests/fixtures/full22.ass         --fonts-dir "$dir" --out "$rep2" --json > "$LOGDIR/font-cli-hit.log" 2>&1         || { say "    缓存第二跑失败：见 $LOGDIR/font-cli-hit.log"; return 1; }
    grep -q '"cache": "hit"' "$rep2"         || { say "    第二跑没吃到缓存，AI5 的债没还上：$rep2"; return 1; }
    local b1 b2 d1 d2
    b1=$(sed -n 's/.*"build_ms": *\([0-9]*\).*/\1/p' "$rep" | head -1)
    b2=$(sed -n 's/.*"build_ms": *\([0-9]*\).*/\1/p' "$rep2" | head -1)
    d1=$(sed -n 's/.*"digest": *"\([0-9a-f]*\)".*/\1/p' "$rep" | head -1)
    d2=$(sed -n 's/.*"digest": *"\([0-9a-f]*\)".*/\1/p' "$rep2" | head -1)
    { [ -n "$b1" ] && [ -n "$b2" ]; }         || { say "    报告里没有 build_ms 分段计时，缓存的账没地方对"; return 1; }
    { [ -n "$d1" ] && [ "$d1" = "$d2" ]; }         || { say "    命中缓存换了渲染身份，缓存就是在改答案：$d1 vs $d2"; return 1; }
    [ "$b2" -lt "$b1" ]         || { say "    命中缓存但建库耗时没降：${b1}ms -> ${b2}ms"; return 1; }
    say "    字体库缓存：全量 ${b1}ms -> 命中 ${b2}ms，两跑渲染身份一致（$d1）"
    local tmp; tmp="$(readlink -f "$LOGDIR")/font-deliver"
    rm -rf "$tmp"; mkdir -p "$tmp/fonts" "$tmp/out"
    local copied=0
    for n in arialbd.ttf arialbi.ttf impact.ttf times.ttf; do
        if [ -f "$dir/$n" ]; then cp "$dir/$n" "$tmp/fonts/" && copied=$((copied + 1)); fi
    done
    [ "$copied" -gt 0 ] || { say "    本机没有 full22.ass 请求的任何字体，交付路径没法验"; return 1; }
    target/debug/subtitletool-cli.exe font scan \
        crates/codec-ass/tests/fixtures/full22.ass \
        --fonts-dir "$(readlink -f "$tmp/fonts")" --no-registry \
        --copy-fonts "$(readlink -f "$tmp/out")" --json \
        > "$LOGDIR/font-deliver.log" 2>&1 || return 1
    [ -f "$tmp/out/manifest.json" ] || { say "    manifest.json 没落盘"; return 1; }
    grep -q '"mode": "copy full fonts"' "$tmp/out/manifest.json" \
        || { say "    manifest 没声明交付档位"; return 1; }
    # 没做 subset 就不能在 manifest 里假装有 subset 哈希
    grep -q '"subset_sha256": null' "$tmp/out/manifest.json" \
        || { say "    manifest 的 subset 一格必须是 null（AG2 未做）"; return 1; }
    local delivered; delivered=$(ls "$tmp/out"/*.ttf 2>/dev/null | head -1)
    [ -n "$delivered" ] || { say "    字体文件没被复制过去"; return 1; }
    cmp -s "$tmp/fonts/$(basename "$delivered")" "$delivered" \
        || { say "    复制出去的字体与源不逐字节相同"; return 1; }
    # --dry-run 时交付也不能碰盘
    rm -rf "$tmp/dry"; target/debug/subtitletool-cli.exe font scan \
        crates/codec-ass/tests/fixtures/full22.ass \
        --fonts-dir "$(readlink -f "$tmp/fonts")" --no-registry \
        --copy-fonts "$(readlink -f "$tmp" )/dry" --dry-run --json \
        > "$LOGDIR/font-dry.log" 2>&1 || return 1
    [ -f "$tmp/dry/manifest.json" ] && { say "    --dry-run 把 manifest 写到盘上了"; return 1; }

    # `--no-font-cache` 那条路必须真的独立：既不读也不写缓存文件，
    # 而与吃缓存的那条给出**同一个**渲染身份（这里用上面那份 4 字体的临时目录，代价小）。
    local p_use p_off c2
    p_use="$(readlink -f "$LOGDIR")/cache-use.json"
    p_off="$(readlink -f "$LOGDIR")/cache-off.json"
    c2="$(readlink -f "$LOGDIR")/cache-off-pair.json"
    rm -f "$p_use" "$p_off" "$c2"
    STK_FONTCACHE="$c2" target/debug/subtitletool-cli.exe font scan         crates/codec-ass/tests/fixtures/full22.ass         --fonts-dir "$(readlink -f "$tmp/fonts")" --no-registry --out "$p_use" --json         > "$LOGDIR/cache-use.log" 2>&1 || { say "    缓存对照跑失败：见 $LOGDIR/cache-use.log"; return 1; }
    rm -f "$c2"
    STK_FONTCACHE="$c2" target/debug/subtitletool-cli.exe font scan         crates/codec-ass/tests/fixtures/full22.ass         --fonts-dir "$(readlink -f "$tmp/fonts")" --no-registry --no-font-cache         --out "$p_off" --json > "$LOGDIR/cache-off.log" 2>&1         || { say "    --no-font-cache 跑不通：见 $LOGDIR/cache-off.log"; return 1; }
    grep -q '"cache": "off"' "$p_off"         || { say "    --no-font-cache 没在报告里留下痕迹：$p_off"; return 1; }
    [ -f "$c2" ] && { say "    --no-font-cache 还是写了缓存文件：$c2"; return 1; }
    local d3 d4
    d3=$(sed -n 's/.*"digest": *"\([0-9a-f]*\)".*/\1/p' "$p_use" | head -1)
    d4=$(sed -n 's/.*"digest": *"\([0-9a-f]*\)".*/\1/p' "$p_off" | head -1)
    { [ -n "$d3" ] && [ "$d3" = "$d4" ]; }         || { say "    不用缓存换了渲染身份：$d3 vs $d4"; return 1; }
    rm -rf "$tmp"
}
start font-scan "17 §2-§4、25 AT-FONT-001/002" "真字体库 + 真 ASS：建库读表而非文件名、六档评级、缺字清单、字体内容变更要失效旧结论"
g_font; rc=$?
if [ $rc -eq 2 ]; then
    report 2 "本机没有可读的字体目录（设 STK_FONTS_DIR）"
else
    report $rc "见 $LOGDIR/font.log 与 $LOGDIR/font-cli.log"
    grep -hE "test result: ok" "$LOGDIR/font.log" 2>/dev/null | sed 's/^/    /'
fi

# ---------------------------------------------------------------- 8c 第三方声明清单
g_notices() {
    command -v python >/dev/null 2>&1 || return 2
    python tools/notices.py --check > "$LOGDIR/notices.log" 2>&1
}
start third-party-notices "21 §11、28 §2" "从 Cargo.lock + registry 缓存生成声明清单，并按 21 §1 白名单校验许可"
g_notices; rc=$?
[ $rc -eq 2 ] && report 2 "没有 python" || { report $rc "见 $LOGDIR/notices.log"; tail -1 "$LOGDIR/notices.log" | sed 's/^/    /'; }

# ---------------------------------------------------------------- 9f 并发与崩溃路径不是"跑一次刚好绿"
# 15 §3 的执行器与 38 §3 的崩溃预算都依赖线程调度：同一份断言在 cargo test 下侥幸通过、
# 在 nextest 下失败过（ISSUES AB2）。所以这里把并发相关的测试重复跑，而不是相信一次运行。
g_soak() {
    # 这里刻意不用 `--retries`：重试用在"抓偶发失败"上是反语义的——
    # 一次红了再跑绿，nextest 报通过，而我们要的恰恰是"只要有一轮红就算红"。
    # （也没有 `--runs` 这个参数，第一版写了它，门阶立刻 FAIL，见 ISSUES AB2 的后续。）
    : > "$LOGDIR/soak.log"
    for round in $(seq 1 12); do
        if ! cargo nextest run -p task-engine >> "$LOGDIR/soak.log" 2>&1; then
            echo "round $round FAILED" >> "$LOGDIR/soak.log"
            return 1
        fi
    done
}
start concurrency-soak "15 §3、38 §3、24 §QA" "执行器/信号量/崩溃预算重复 12 轮，抓调度相关的假绿"
g_soak; rc=$?
[ $rc -eq 0 ] && grep -E "^ *Summary" "$LOGDIR/soak.log" 2>/dev/null | tail -1 | sed 's/^/    /'
report $rc "见 $LOGDIR/soak.log"

# ---------------------------------------------------------------- 9 文档声明核对
g_docs() {
    command -v python >/dev/null 2>&1 || return 2
    python tools/doc_claims_check.py > "$LOGDIR/docs.log" 2>&1 || return 1
    # 第二段：文档点名为证据的测试，必须在**本次**门阶的执行记录里真的跑过。
    # 只查"名字存在"是不够的——一条从没被执行过的测试同样能给文档当证据（ISSUES AG17）。
    # 放在 `tests` 阶之后跑，读的就是这一轮写的 tests.log，不是上次的残留。
    python tools/audit_evidence.py >> "$LOGDIR/docs.log" 2>&1
    # 第三段：问题清单与交付状态必须互相自洽（ISSUES §AJ1）。
    # 前两段查的是"文档说的东西存在吗、跑过吗"，这一段查的是"两边说法一致吗"：
    # 已解决的事不许还留在「没做」清单里，开着的事必须在状态文档里点得到名。
    python tools/issue_status_audit.py >> "$LOGDIR/docs.log" 2>&1
}
start doc-claims "39 §清单、22 §诊断、24 §QA" "文档点名的测试/标识符必须真的存在且本次真的执行过；问题清单与交付状态不得互相矛盾"
g_docs; rc=$?
[ $rc -eq 2 ] && report 2 "没有 python" || report $rc "见 $LOGDIR/docs.log"
grep -E "核对|DOC CLAIMS|已执行|点名|EVIDENCE AUDIT" "$LOGDIR/docs.log" 2>/dev/null | sed 's/^/    /'

# ---------------------------------------------------------------- 10 release 构建
if [ "${RELEASE:-0}" = 1 ]; then
    g_release() {
        cargo build --workspace --release > "$LOGDIR/release.log" 2>&1 || return 1
        [ -f target/release/subtitletool-cli.exe ] || return 1
        # 28 §2 把 renderer-host 列为独立产物：没有它，"DLL 不进主进程"就只是一句文档
        [ -f target/release/subtitletool-render-host.exe ] || {
            say "    28 §2 要求的 renderer-host 产物没被构建出来"
            return 1
        }
    }
    start release-build "28 §2" "cargo build --workspace --release 产出 CLI 与 renderer-host 两个可执行文件"
    g_release; report $? "见 $LOGDIR/release.log"
else
    say ""
    say "=== release-build  [28 §2]"
    if [ "$STRICT" = 1 ]; then
        say "    --strict 下必须跑 release 构建（RELEASE=1）"
        printf '%-30s FAIL  RELEASE=1 未设置（strict）\n' release-build >> "$SUMMARY"
        FAILED=$((FAILED + 1))
    else
        say "    SKIP：默认不跑（慢）。发布前用 RELEASE=1 bash tools/check_all.sh --strict"
        printf '%-30s SKIP  RELEASE=1 未设置\n' release-build >> "$SUMMARY"
        SKIPPED=$((SKIPPED + 1))
    fi
fi

# ---------------------------------------------------------------- 摘要
say ""
say "==================== GATE SUMMARY ===================="
cat "$SUMMARY"
say "fail=$FAILED skip=$SKIPPED strict=$STRICT"
if [ "$FAILED" -gt 0 ]; then
    say "RESULT: FAIL"
    exit 1
fi
if [ "$SKIPPED" -gt 0 ]; then
    say "RESULT: PASS WITH SKIPS（发布机请加 --strict）"
    exit 0
fi
say "RESULT: PASS"
exit 0
