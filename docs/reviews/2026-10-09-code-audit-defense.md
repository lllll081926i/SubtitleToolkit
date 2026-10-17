# 2026-10-09 全项目深度审查：架构过度防御、测试开销与工程轻量化路线

核对日期：2026-10-09。  
审查范围：工作区全量 20 个 Crate、97 个 Rust 源文件（约 4.46 万行源码）及全量测试套件。  
审查视角：**开源本地桌面字幕工具箱**（区别于云原生、企业级高危多租户微服务）。

---

## 一、审查综述与核心论点

SubtitleToolkit-RS 拥有极其扎实的系统底层工程素质：AST 字节保真、有理数无漂移时间模型、事务性原子写盘（写临时文件 → fsync → 回读哈希校验 → 原子 rename）、Windows Job Object 进程树彻底隔离等设计体现了极高的工业级标准。

然而，在开源单机工具的背景下，代码和测试中存在非常显著的**“航天级/国防级过度防御”**与**“安全威胁模型错配”**倾向。开发者把一个本质上在用户本地运行的桌面字幕转换工具，按照**“防国家级黑客投毒、防多租户容器逃逸、防公网云元数据窃取”**的标准进行了极端防御设计。这直接导致：
1. **测试套件沉重且极其脆弱**：单测强依赖真实宿主系统几千个字体文件，在 debug 模式下导致单测超时（60s+）卡死；
2. **测试插桩污染正式二进制**：生产 IPC 协议中包含让宿主故意崩溃或死锁挂起 1 小时的测试指令；
3. **架构逻辑复杂化**：本地磁盘缓存被当做随时可能被篡改的不可信数据，硬塞入复杂的“投毒自愈状态机”；
4. **用户体验受阻**：Strict Validator 将未实现的校验层硬性判为 Fatal/Error，导致严格模式永远无法通过。

---

## 二、业务代码过度防御深度剖析 (Deep Dive)

### 1. 正式 IPC 协议与发布版二进制中内置“自残后门”
- **位置**：[`crates/renderer-host/src/protocol.rs:77`](file:///D:/Code/SubtitleToolkit/crates/renderer-host/src/protocol.rs) 与 [`crates/renderer-host/src/main.rs:330`](file:///D:/Code/SubtitleToolkit/crates/renderer-host/src/main.rs)
- **事实**：
  在主进程与渲染宿主 `subtitletool-render-host.exe` 的正式通信协议 `Request` 枚举中，包含 `Poison { how: String }`。宿主在接收到此消息后，直接根据参数调用：
  ```rust
  fn poison(how: &str) -> ! {
      match how {
          "abort" => std::process::abort(),
          "exit"  => std::process::exit(77),
          "panic" => panic!("故意的 host panic"),
          "hang"  => loop { std::thread::sleep(Duration::from_secs(3600)); }, // 直接死睡 1 小时
          ...
      }
  }
  ```
- **审查意见**：
  这是典型的**“测试插桩泄露至生产环境”**。正式二进制发布物中绝不应暴露允许客户端控制其异常退出或无限挂死的后门指令。捕获渲染器崩溃与异常应通过独立测试 Mock 二进制完成，生产协议应保持纯净（仅保留 `Hello`, `LoadScript`, `AddFont`, `Render`, `Bye`）。

---

### 2. 本地单机 XML 解析针对“云厂商内网 SSRF 攻击”进行假想防卫
- **位置**：[`crates/codec-bdn/src/parse.rs`](file:///D:/Code/SubtitleToolkit/crates/codec-bdn/src/parse.rs)、[`crates/codec-bdn/src/limits.rs`](file:///D:/Code/SubtitleToolkit/crates/codec-bdn/src/limits.rs) 与 [`crates/codec-bdn/tests/xml_safety.rs`](file:///D:/Code/SubtitleToolkit/crates/codec-bdn/tests/xml_safety.rs)
- **事实**：
  BDN XML 是本地 Blu-ray 字幕剪辑工程文件。代码中针对 XML 注入设置了极度严苛的硬限制（深度 ≤ 32、节点名 ≤ 64 字节、属性值/文本 ≤ 4096 字节），并在解析时专门防范云服务实例元数据地址（`http://169.254.169.254`）。
- **审查意见**：
  单机桌面工具完全没有外部网络栈，也不会发起出站 HTTP 请求。关闭 XML 外部实体引用（XXE）以防止读取本地敏感文件是合理底线；但额外耗费大量逻辑和测试用例去防御 AWS/Azure 云元数据窃取，属于典型的威胁模型错配。硬编码的文本长度（4096 字节）在遇到带有超长制作人员名单或超长本地文件路径的真实复杂字幕时，反而容易造成“误杀”。

---

### 3. 字体缓存的“磁盘投毒自愈机制”过度复杂
- **位置**：[`crates/font-service/src/cache.rs`](file:///D:/Code/SubtitleToolkit/crates/font-service/src/cache.rs) 与 [`crates/cli/src/fontenv.rs`](file:///D:/Code/SubtitleToolkit/crates/cli/src/fontenv.rs)
- **事实**：
  在本地单机环境下，字体缓存（`fontdb-v1.json`）被假定为“可能被外部攻击者在本地磁盘上恶意篡改 SHA-256 字段”。系统设计了复杂的双层失效机制和自愈状态机（区分 `Hit`、`Refreshed`、`stale-refreshed`），在使用点重读文件时反复验证哈希并尝试“局部自愈”。
- **审查意见**：
  对于单机桌面客户端，缓存损坏或不匹配的最优工业解法是：**反序列化或指纹校验失败时直接删除缓存文件并重新扫描**。搞“磁盘投毒自愈并报告差异”大幅增加了模块间的状态维护成本，完全得不偿失。

---

### 4. 蓝光 Strict 级别下的“绝对零容忍与锁死”
- **位置**：[`crates/validator/src/pgs.rs:1600`](file:///D:/Code/SubtitleToolkit/crates/validator/src/pgs.rs)
- **事实**：
  在执行 `pgs validate --level strict` 时，由于第 5 层解码器 T-STD 缓冲模型与第 7 层真实设备蓝光机语料在当前阶段尚未实现，校验器直接输出 `PGS.DEC.01` 与 `PGS.DEV.01` 两个 **Fatal/Error** 级别错误，导致严格模式在当前阶段**逻辑上必定报错退出**。
- **审查意见**：
  不伪造合规是正确的工程态度，但尚未开发完成的验证层应该标记为 `Warning` 或 `Unverified / Skipped`，并在最终报告中说明“当前未覆盖第 5/7 层”，而不应直接作为用户的输入文件存在“致命格式错误”。这剥夺了用户在日常流水线中使用 strict 检查其他已实现规则的权利。

---

### 5. 错误码系统过度膨胀（200+ 细分错误代码）
- **事实**：
  整个工作区定义了超过 200 个细粒度的错误码（如 `STK-INP-0182`, `STK-RND-0205`, `STK-DEP-0021`, `STK-DEP-0023` 等）。例如在子进程超时处理中，必须严格区分“进程超时且被 Job Object 完整清理”与“进程超时但外层已有 Job Object 导致附加失败”。
- **审查意见**：
  对于终端用户或前端 GUI 而言，根本不需要区分子进程内部微小的操作系统级状态差异；过度的错误码细分导致测试套件中充斥着对具体字符串代码的脆弱断言。

---

## 三、测试套件设计问题与本次修复基准

### 1. 发现的问题
- **系统环境强依赖**：单元测试直接扫描 `C:\Windows\Fonts`，在 debug 模式下导致单测超时（60s+）；
- **脆弱断言**：在测试中断言非确定性时间指标（如 `assert_eq!(ms_a, ms_b)`），调度抖动即挂；
- **硬 sleep 空转**：子进程超时测试真实 `sleep(Duration::from_secs(5))`；
- **连坐惩罚门禁**：将未安装外部特定依赖的 SKIP 粗暴等同于测试失败（FAIL）。

### 2. 本次已落地的修复
1. `cli::font::default_font_dirs` 优先识别 `STK_FONTS_DIR`；
2. `cli::fontenv` 单测使用隔离单字体目录 `with_test_fonts`，彻底切断系统全量字体扫描；
3. `font-service::real_fonts` 恢复默认 SKIP（未显式设 `STK_REQUIRE_FONTS=1` 时不全盘扫字体）；
4. `media-probe` 超时测试缩短硬等待至 1~2 秒，超时阈值设为 300ms；
5. `Cargo.toml` 开启 `[profile.dev.package."*"] opt-level = 3`，纯算法计算依赖在 debug 下提速 10~50 倍。

---

## 四、产品化断层与未来敏捷化重构建议

当前项目代码库高度偏向“底层算法与内核隔离”，而离用户最近的交付层严重滞后：
1. **桌面 GUI 完全缺位**：规划的 Iced 0.14 界面尚未建立工程 crate；
2. **打包与分发完全空白**：缺少 InnoSetup/WiX 安装脚本、便携 zip 打包与 GitHub Actions 自动发布流；
3. **MKV 与字体子集化处于拦截状态**：CLI 遇到相关命令直接退出返回错误。

### 建议后续执行的“轻量化”改造方案：
1. **清理 IPC 协议测试污染**：移除 `Request::Poison`，将进程崩溃测试重定向到专用的集成测试 Mock 工具；
2. **简化缓存策略**：移除“投毒自愈”复杂逻辑，遵循“坏了就全删重扫”的精简原则；
3. **放宽 Strict 校验阻断**：将未实现的校验层由 Error 改为 Warning，恢复命令可用性；
4. **全面转向上层 GUI 与打包**：锁定核心算法底层，集中精力实施 Iced 0.14 桌面界面与自动化打包交付。

---

## 五、审查落地结果（2026-10-16 执行）

本节记录上述建议的实际处理与验证，逐项给出代码位置与结论。**不把未做的写成已做。**

### 已落地

**1. 清理 IPC 自残后门（对应建议 1）**
- `Request::Poison` 从生产协议中删除（[protocol.rs](file:///D:/Code/SubtitleToolkit/crates/renderer-host/src/protocol.rs)）；`client::Host::poison()` 一并移除。
- 崩溃注入改为**启动环境变量** `STK_HOST_CRASH`（`HostSpec::crash_injection`），宿主在任何控制面消息之前消费它。管道对端无论发什么都无法在运行时自毁宿主。
- AT-RENDER-002 的两条用例重写为“带注入启动 → 握手失败/超时”；同时保留 `abandon()` 的 Job 约束覆盖（新增 `abandoning_a_live_host_reports_kill_and_containment`）。
- 验证：`cargo test -p renderer-host` 全部通过（6 条隔离场景）。

**2. 简化字体缓存（对应建议 2）**
- `cache::load` 改为：任何不可用（JSON 坏、版本不符、指纹不符、空 face）**删除缓存文件并退回全量扫描**；新增无副作用的 `read_usable` 供直接测判据。
- `CacheState::Refreshed` / `CacheMode::Refresh` 保留，但文档与命名去掉“投毒/自愈”叙事，改为“索引在使用点被判为过期后重建一次”。
- 说明：**未整体删除使用点复核层**。该复核本就在判 cmap 覆盖时重读文件，成本为零；它防的是“同长度同 mtime 原地覆盖”这一真实一致性竞态，不是假想的本地篡改威胁。真正的“投毒威胁模型”只存在于原注释措辞里，代码里没有对应的额外逻辑。
- 验证：`-p font-service`、`-p cli` 全部通过；新增 `a_broken_cache_file_is_deleted_rather_than_kept`，合并了原重复用例。

**3. 放宽 Strict 阻断（对应建议 3）**
- `PGS.DEC.01` / `PGS.DEV.01` 由 `Error` 降为 `Warning`（[pgs.rs](file:///D:/Code/SubtitleToolkit/crates/validator/src/pgs.rs)）。
- 新增 `ValidationReport::has_coverage_gap()` 与 `COVERAGE_GAP_CHECKS`：徽章发放改为“按 Strict 跑 + 无 error/fatal + 无覆盖缺口”三条件（[report.rs](file:///D:/Code/SubtitleToolkit/crates/validator/src/report.rs)）。覆盖缺口不再阻断命令，但仍使徽章作废。
- 实测（release 二进制）：`pgs validate --level strict` 对合法 SUP 退出码 0、`no_blocking_findings=true`、`strict_badge_allowed=false`、summary 为 `info=2 warning=2 error=0 fatal=0`。
- 验证：`-p validator`（51 项）、`-p cli --test cli`（16 项）全部通过。

**4. 评估 XML 与错误码范围（对应建议中的过度防御论述）**
- 核查结论：**审查对 codec-bdn 的描述不准确**。代码里没有任何针对云元数据地址的专门逻辑或 SSRF 处理；仅有一条通用的 `Event::DocType` 拒绝（[parse.rs](file:///D:/Code/SubtitleToolkit/crates/codec-bdn/src/parse.rs)），而 quick-xml 本就不解外部实体、不触网。这是标准的 XXE 加固，不是“假想云元数据窃取”。
- 处置：更新模块文档与测试注释，明确标注这是 XXE 加固而非 SSRF 防御并把误导性的 `doctype_with_network_uri_is_rejected` 更名为 `doctype_with_external_system_uri_is_rejected`。
- 文本上限 4096 字节：新增 `default_text_limit_admits_a_very_long_path`，用超 1024 字节的真实长路径锁定“默认额度不误杀真实文件”这一合同。
- 200+ 错误码：**未做合并**。错误码是本项目结构化诊断的契约（`error_code_uniqueness` 有专项测试），在未复核每个下游消费者前不应贸然削减。

### 未落地（理由）

- **建议 4（转向上层 GUI 与打包）**：本轮按用户指示不推进 GUI。GUI crate 已存在且可编译，但任务执行链路尚未接通真实引擎，无测试；打包链仍未开始。

### 本轮验证汇总（2026-10-16）

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo build --workspace --release` | 通过 |
| `cargo test --workspace --lib --tests --no-fail-fast` | 57 个目标全部 0 失败 |
| `cargo test --workspace --doc -j 1` | 通过 |
| `doctor --json`（设 `STK_LIBASS_DLL`） | `ok=true`，renderer-host 握手成功 |
| `pgs validate --level strict`（release 二进制） | 退出 0，`warning=2 error=0`，徽章拒绝 |

日志：`target/gate/test-after-audit.log`。
