from pathlib import Path
import re,json,zipfile,hashlib,datetime
src=Path('/mnt/data/subtitle_tool_rust_planning_v3_1')
out=Path('/mnt/data/subtitletool_full_audit_20261008')
files=sorted(src.glob('*.md'))
issues=[
('A001','P0','多规格共享渲染边界错误','07:66-72;09:67-75;13:109-114;40:51-68','统一时间点并集不等于不同 Profile 的 RGBA 可共享；不同 canvas、renderer、HDR、字体环境、storage/PAR 的结果通常不同。','引入 RenderGroupKey=(script transform,renderer id+version,canvas,storage,PAR,font bundle,color policy,animation policy)，每组内部取时间采样并集；共享 AST/时间轴/可证明相同的 RGBA，跨组不共享帧。','720/1080/2160与libass/VSFilter输出不错误缓存；相同组 BDN+SUP只渲一次。'),
('A002','P0','HDR亮度算法缺失','11:1-37;05:24-31;36:11-31','只有“百分比/203 nit”概念，没有色域、EOTF/OETF、输入编码假设、白电平映射公式、alpha与明度边界等。ASS 本身不能保证播放端真实 nit。','明确模式 1 字节级 legacy dim；模式2 在指定假设下做 sRGB解码->线性亮度目标->sRGB编码并量化；HDR/PQ目标需将最终视频合成链作为前置条件；所有模式保留alpha并追踪内联颜色和\\t嵌套颜色。','白灰彩/描边/动态变色完整单元黄金样本+跨播放器视觉对比；UI不得承诺绝对nit。'),
('A003','P0','PGS颜色与时序验收的形式化规范缺失','13:8-120;10:20-28;25:15-19','有字段清单却没有完整字节契约、epoch/display-set状态图、timestamp wrap/排序/允许FPS矩阵、来源规范可审计表；Strict尚不可复现。','分离结构解析验证、显示时序验证、buffer模拟器、设备实验；固定含PDS 5字节调色板条目(index,Y,Cr,Cb,alpha)的参考向量；明确90k tick/32位封装、DTS策略和分片。','与FFmpeg/其他独立decoder双向解码，golden逐段断言，错误码可定位offset。'),
('A004','P0','WBS DAG 缺节点且依赖方向错误','27:1-69;26:1-39','RenderTimelinePlanner、encoding、renderer能力检测、process runner、color matrix、字体许可等没有任务ID；Strict validator依赖Advanced Author不合理。','新增 R040 Timeline、C005 output color policy、J006 Planner DAG、V001 structural validator、V002 timeline、V003 buffer、A000 external process runner等；Strict validator的单项可先于Advanced Author开发。','机器检查任务ID唯一、所有依赖均存在、无环、每阶段有gate/test/交付件。'),
('A005','P0','完整16:9硬规则有例外冲突','00:17-23;09:3-11;09:86-97;32:12-14','09允许“实验性非16:9画布”，但用户对特效输出要求完整16:9；原始ASS的PlayRes不应被视为强制16:9。','特殊字幕输出目标画布仅允许16:9；原始脚本PlayRes/LayoutRes保真，不用强制改写源；其他长宽比单独作为非特效/不支持实验域并与正式工作流分离。','2.39:1黑边、PlayRes非16:9输入在全画幅16:9目标上保持正确位置。'),
('A006','P0','字节级ASS保真与自动改写规则未固化','08:5-18;14:3-10;25:3-4','未知tag保留已规定，但源编码、BOM/换行、注释顺序、字段动态映射、损坏输入恢复、内联\\t和\\rStyle等语义没有可执行契约。','建立 TokenSpan AST 和ByteRangePatch；源文件无修改时逐字节等价；变换仅改已识别 token；不支持的未知颜色扩展警告而不擅改。','真实ASS corpus逐字节Roundtrip(无变换)及字段差异白名单；编码异常拒绝隐式有损替换。'),
('A007','P0','特效渲染兼容清单及回退规则不足','12:33-44;12:64-112;24:20-21','CSRI/VS/AviSynth接口识别正确，但没有实际测试的provider二进制/导出表/调用约定矩阵；“自动选择”易给出错误兼容承诺。','Phase0对每个支持的provider锁文件SHA和x86/x64；CapabilityManifest列 APIs/输出格式/字体加载/线程安全/ANSI限制；无合适provider则失败或显式用户确认fallback。','每个provider独立宿主进程运行smoke + differential + crash isolation测试。'),
('A008','P0','严格兼容等级夸大风险','02:49-53;13:87-120;25:19','“Level5+device corpus通过→Strict”没有定义真实硬件覆盖、认证机构、正式约束引用；样本库通过不代表所有设备必然兼容。','UI采用“通过本工具严格校验（模型/版本）”；仅经过真实机型复验者标“实机验证”；禁止“认证/保证所有蓝光设备”。','报告列validator版本、profile、测试设备/固件/样本及已知未覆盖限制。'),
('A009','P0','安全：独立进程不等于沙箱','12:45-55;23:17-21','Job Object主要提供进程树资源管理，并不阻断第三方恶意DLL对用户文件/网络的访问。','将renderer隔离明确为崩溃隔离；评估restricted token/low integrity/AppContainer及插件资源限制和权限最小化；不可信DLL须显式用户同意。','恶意plugin访问非授权路径、外联、生成子进程等威胁模型与可执行测试。'),
('A010','P0','质量门槛未量化','24:1-75;25:1-28','只有AT-ID一行说明，没有 fixture、输入/预置、步骤、期望数值和容差；无法作为验收测试脚本。','定义每项 TestCase={fixture,setup,steps,expected,oracle,tolerance,env,artifact,owner}; 按P0/P1 release gate排序。','CI能逐条产生机器可读测试结果、视觉diff报告和基准版本。'),
('A011','P1','文档次序不够合理','00:24-90;35:1-63;36:1-41;40:1-192','01-34依顺序排列，35-41后补的UI/Schema/源码审计影响早期定义，但仍置于附录，容易漏读；无最终发布时点的总索引。','保持物理文件名以免破坏引用；增加按依赖的READER_ORDER及最新判定覆盖表，以功能主题整合新旧章节。','所有文件在索引中唯一且链接有效；冲突结论有唯一来源。'),
('A012','P1','CLI与进程产品命名冲突','07:5-11;20:1-24;28:9-18','GUI在系统架构/发布称SubtitleToolkit.exe，而CLI文档第一句称subtitletool.exe(GUI)。','统一GUI SubtitleToolkit.exe、CLI subtitletool-cli.exe、hosts renderer-host-{arch}.exe；固定AppId、安装目录和log prefix。','生成产物清单与CLI --version所有名字一致。'),
('A013','P1','Windows 10生命周期策略过时','00:1-8;06:15-19;24:35-36;28:1-38','Win10 22H2已于2025-10-14结束常规支持。文件视Win10/Win11为等价发布基线而未提ESU/老系统维护义务。','Windows11为主要受支持系统，Win10 22H2设兼容/尽力支持并说明ESU；如坚持全面支持，维护独立测试与安全告知。','产品支持矩阵/installer requirements/help/CI一致。'),
('A014','P1','PGS fast-path“无损”措辞不准确','02:42-47;13:23-43;05:60-65','保留ODS仅能保证不重编码位图；调色板、位置、时间修改当然会改变视觉/播放表现，不应称为总体无损。','统一UI标签“位图不重编码”；验证ODS字节不变仅作为必要条件，另检查其它segment副作用。','Loss report将结构保真、视觉变化和符合性分成独立维度。'),
('A015','P1','多输出预设缺继承/冲突处理','09:55-66;15:1-36;36:10-41','Preset只有示例，没有参数规范、条件禁用、用户覆盖、版本迁移细则；同名路径输出覆盖冲突缺模拟。','定义强类型JsonSchema/TOML schema、字段优先级及迁移、dry-run collision planner，单个job所有输出目标唯一，源文件默认不可覆盖。','随机组合Profile property tests；路径冲突预检、非法/保留名、不同大小写碰撞。'),
('A016','P1','输出事务跨卷与多文件目录缺口','07:81-83;18:19-20;38:1-26','atomic rename不是任意跨卷/目录都原子；BDN是XML+PNG多文件集合，MKV工具有大文件输出；仅“写tmp→rename”不完整。','每个输出在最终目标同卷 staging；多文件BDN按目录发布和manifest；Windows ReplaceFile/MoveFileEx异常及占用文件fallback需实测。','断电/磁盘满/杀进程模拟不得留下冒充成功的产物，旧文件完整保留。'),
('A017','P1','ASS→SRT剥离语义不足','14:3-10;08:43-51','\\N/\\n/\\h、drawing文本、k标签、嵌套事件、重叠对白、重号等无法靠“remove tags”保证语义。','实现文本投影状态机，明确绘图丢弃/格式保留/注释策略/重叠策略并输出逐项LossReport；绝不把ASS绘图指令当台词。','用带转义/绘图/双语/特效字幕样例验证产出的SRT语义。'),
('A018','P1','编码探测与原始字节保存策略缺失','05:10-22;08:8-18;14:6-7','编码选择仅列控件，缺UTF-8 BOM、UTF-16、GB18030、Shift-JIS、错误解码回退、混合行编码定义。','探测仅给候选及置信度；用户可选解码并预览异常数；不能默认replace invalid bytes后覆写。','中日韩+混合BOM/乱码corpus，保存后字段未修改处等价。'),
('A019','P1','字体打包缺字体授权限制','17:1-83;21:1-50','具备识别/子集/附件功能，但未核查字体嵌入权限、字体许可、Windows系统字体分发限制；技术上能提取不代表可以再分发。','区分个人本地使用和公开分享；检查OS/2 fsType并结合字体EULA；对未知授权字体仅报告/不默认嵌入分发。','字体清单增加嵌入许可状态和用户确认日志。'),
('A020','P1','PGS帧率/视频规范与自定义输出合规矩阵不完整','03:3-30;05:40-58;13:87-107','列出多种FPS/尺寸但未为BD/UHD BD规范列合法组合；仅禁用2160并不保证剩余组合严格合规。','按 Format+Profile+Resolution+FrameRate+Colorimetry+Output 定义能力矩阵，UI动态禁用非法组合，标来源与版本。','每个输出Profile组合都通过合法/非法 golden vectors。'),
('A021','P1','SUP原始色彩空间推定过强','11:21-23;13:40-43;16:10-21','SUP文件没有充分明确标注BT.709/BT.2020 PQ等视频完整色彩语义，单凭尺寸/调色板或文件扩展名不能可靠推断。','要求用户指定源/目标颜色策略或从关联视频有证据推断，记录置信度；不明时禁用“精确HDR映射”，保留百分比实验模式。','相同字节SUP分别在709/2020假设下输出可重放结果；报告输入假设。'),
('A022','P1','HFR采样与降采样算法不可执行','09:67-75;12:87-101;13:109-114','“事件精确边界+逐帧/自适应”没有给sampling条件、静态检测、最大预算、帧尾闭区间和降帧准则，动态特效将性能爆炸。','定义时间域为半开区间[start,end)，按输出分组，帧网格+可观察边界；动态帧率降采样用像素差/可见保持时间/带宽预算约束，默认只采取保真路径。','动态移动、\\t、karaoke在12/24/60fps的视觉误差和速度基准。'),
('A023','P1','MKV替换轨道流程缺命令级契约','18:1-20;14:30-31;05:83-88','目前只有“extract→remux”文字，未明确定义track id、language/IETF BCP47、default/forced、附件、章节、音轨/视频码流保留策略。','固定mkvmerge --identify -J/mkvextract tracks/attachments/mkvmerge构建参数数组和预检/验证策略；以新MKV为输出，源MKV不改。','有多个字幕轨/附件/章节/相同track names时，新文件除目标变化外元数据保持。'),
('A024','P1','批量资源容量模型缺阈值','15:14-36;37:1-32','有池和内存估计，但缺限流公式/磁盘空闲策略/压缩缓存生命周期/中途暂停恢复序列化的版本兼容性。','给出GlobalMemoryBudget、per-host maxframes、host IPC shared mem上限和cache quota，负载下backpressure，恢复artifact版本清单。','100文件×多profile稳定运行，内存不无界，重启跳过已验证节点。'),
('A025','P1','进度/取消协议定义不足','07:74-83;12:45-55;15:14-36;38:23-26','取消说协作式，但renderer-host、child process、pipeline输出事务状态转换未定义；任务提交/重试存在重复输出风险。','定义Queued/Preparing/Running/Cancelling/Verifying/Committed/PartialSuccess/Failed状态机及幂等job key，取消传递与超时升级顺序。','kill+cancel+retry下无覆盖原文件、无未释放shared memory。'),
('A026','P1','外部依赖缺可复现锁与获取策略','06:27-43;21:1-50;30:1-51;40:184-192','调研列项目/主分支URL但没有 commit/release hash、Windows binary来源、build flags、license/SPDX、ABI测试结果。','建立DEPENDENCY_LOCK与源码参考LOCK；所有特定声明绑定tag/commit；公司/个人分发模式拆分所需通知及来源许可证。','在全新Windows VM中按锁文件复制构建/运行/验证。'),
('A027','P1','许可证：外部GPL/AGPL和工具分发边界','21:1-50;28:11-18','已识别风险但还未决定最终产品许可证、Bundled vs User-Supplied、源码提供义务；MkvToolNix是GPL项目。','发布前冻结license policy；ffprobe按构建配置分LGPL/GPL；MKVToolNix/VSFilterMod/ALASS分别审计独立分发，外部进程非法律豁免。','THIRD_PARTY_NOTICES、SPDX SBOM、下载来源+hash+source-offer产出。'),
('A028','P1','软件首发范围和阶段门槛不一致','01:79-83;03:3-30;26:1-39','P0/P1多处“首发 是”，但前期技术验证、后期蓝光strict、可选legacy/OCR等缺V1.0 cutline；易出现首发计划无限扩张。','定义内部Milestone M0原型/M1文本工具/M2渲染+BDN/M3PGS/滤镜稳定版/M4严格蓝光高级版，与“首发”独立管理。','每阶段有范围、依赖、验收、非目标、失败退路。'),
('A029','P1','UI细节仍是控件清单而非完整规格','04:12-63;05:1-110;35:1-63','缺列自适应、屏幕分辨率、键盘焦点/IME/屏读/无障碍、批量参数继承、验证失败提示、具体交互状态和样例截图；最小1100×720对低分屏不友好。','补逐页交互合同：Input/Button-Message-Update-Task-Result、默认值、启用条件、错误文案、可访问性/缩放、响应式收纳；先建Iced组件story harness。','1366×768/1080P/4K及100%-200%DPI场景截图、键盘全操作、E2E断言。'),
('A030','P1','错误码与日志没有强类型目录','22:1-19;35:27-32;38:1-26','STK-PGS-0104只是示例，缺全域code列表、JSON schema、隐私字段分类、路径脱敏、撤销/恢复动作，diagnostics无法聚合。','定义ErrorCode registry、severity、stage、retryable、causes、user_message、dev_info、redaction rules；只按用户明确操作附原文件。','每个失败路径映射唯一错误码并落日志/诊断包；脱敏快照测试。'),
('A031','P1','源代码审计不可复现','29:161-219;30:1-51;40:1-19;40:184-192','上次Web源代码检视没有commit SHA、文件hash、验证命令输出和实际跑的测试；不能作为第三方依赖已正式“严格验证”的证据。','SOURCE_AUDIT_LOCK.json记录仓库、commit、file path、raw blob hash、API/许可证文件、审计日期；本地clone后crate/test+Windows运行日志入库。','独立开发机器可按lock复现静态分析和具体结论。'),
('A032','P1','Safety Normalization范围含糊','01:30-42;03:19-26;14:1-31','“Safe Normalize”未区分排序、重复删除、换行、最短时长修正、强制字幕标记等可能改变观看语义的操作。','分为Byte-safe / Semantic-preserving / Content-altering三级；后两级默认关闭并有预览、可还原策略。','自动操作前后event/text/style/PTS hash维度报告；每项可单独回退。'),
('A033','P2','自用与公开分发目标边界缺失','01:1-83;21:1-50;28:1-38','产品说个人使用，不等于永不公开分发；是否闭源/开源/公开下载决定各依赖许可、签名/更新、法务安排。','在ADR新增Licensing/Distribution Goal：private only / public open source / public proprietary，临时默认不要捆绑GPL组件。','发布检查表按三种分发策略生成不同gate。'),
('A034','P2','字库/动画基准缺合法语料与可分享边界','17:45-61;24:1-75','真实动漫 ASS、商业字体和蓝光PGS参考可能受版权限制；如果测试样本直接打包公开，存在许可风险。','Corpus分公开合成fixture/授权真实fixture/本地private fixture；公开测试只放权属明确材料与最小合成向量。','仓库manifest明确来源/许可证/是否可公开分发。'),
('A035','P2','应用视觉品牌/设计系统未规范','04:1-63;05:1-110;35:1-63','有页面和系统色切换，但缺Design tokens、层级、间距、Typography、状态色、图标、密度、动画减弱规则、窗口调整行为。','补DESIGN_SYSTEM和COMPONENT_CATALOG；克制动效，以性能与可诊断性为先。','设计token映射所有页面/主题；视觉回归截图审核。'),
('A036','P2','变更追踪只有版本日志','31:1-69;34:1-14;41:1-12','没有单一需求ID->设计->代码->测试关联，不能证明“所有UI选项都实现”；变更日志没有每项修复owner和验收。','建立Traceability Matrix，REQ-xxx→UI control→Config→Stage→WBS→Test→Release；状态Proposal/Accepted/Implemented/Verified。','自动扫描所有正式UI动作可追到至少一个测试与处理stage。'),
('A037','P2','VFR和字幕FPS概念提醒可再强化','10:1-37;05:33-39','文档合理地区分speed/snap但未定义22.976? 小数显示与真分数，VFR帧时间映射、多个视频段变速精度。','FPS显示分数+友好小数；时间映射用i128防溢出，Snap必须给frame time map；未获得VFR timecode map时拒绝“真实逐帧贴合”。','2小时23.976与24累计漂移、VFR剪辑、33-bit边界等特定fixture测试。'),
('A038','P2','ASS合并的样式引用不完整','14:24-26;08:8-18','Style重名处理写了Dialogue.Style，却没提内联\\rStyle、Script Info依赖、Layer冲突、Fonts/Graphics附件重名。','合并时建立per-source StyleMap，在Event.Style和Override \\rName中双向替换；多脚本属性冲突生成报告/拒绝自动合并。','两个ASS共享Style名但字体不同/含\\rStyle时合并渲染视觉等价。'),
]
bydoc={
'00':('补充 V3.1 42份的正式索引、跨章节覆盖优先级、阅读顺序与审计日期；不要把旧V3文字称最新版。','A011,A026,A036'),
'01':('明确首发里程碑范围、Windows11主支持/Win10兼容、Strict认证措辞和产品非目标。','A008,A013,A028,A033'),
'02':('对每一用户流补输入格式/手动参数/失败分支/产物与损失说明、目标视频辅助规则。','A015,A020,A032'),
'03':('将功能行增加稳定REQ ID、release里程碑、功能完成定义、依赖主键、测试ID。','A028,A036'),
'04':('补响应式主窗口布局、1366×768缩放、键盘/焦点/屏读无障碍与拖放状态。','A029,A035'),
'05':('每个控件补默认值、启用/禁用条件、验证、事件、任务行为与错误信息。','A002,A029,A035'),
'06':('锁 Iced 0.14 release tag 而非master；Win10生命周期更新；正式描述依赖二进制来源与InnoSetup许可。','A013,A026,A027'),
'07':('将 multi-output DAG按 RenderGroupKey 分组；加入任务接口、job plan hash、process sandbox与同卷输出事务。','A001,A009,A015,A016'),
'08':('定义ASS ByteRangePatch/编码策略、PGS segment typed IR、未知字段透传和精确时间类型。','A006,A017,A018'),
'09':('删特效输出的非16:9例外；区分统一时间轴与每配置渲染组；多输出颜色/renderer共享范围。','A001,A005,A022'),
'10':('公式边界条件、i128中间值、时间原点、PTS/DTS互斥/相关处理、VFR及误差上限。','A003,A022,A037'),
'11':('补两套亮度算法正式公式、色域假设、203nit限制、HDR/SDR SUP source policy、色彩测试向量。','A002,A021'),
'12':('建立provider二进制兼容清单、ABI probe具体步骤、权限边界与渲染命令字节协议。','A007,A009,A026'),
'13':('补PGS PDS/ODS/PCS字段表、时间/颜色/FPS合法组合、decoder buffer模型、合规证据边界。','A003,A008,A014,A020'),
'14':('ASS→SRT绘图/转义/多事件策略，BDN PNG格式独立策略，merge内联样式引用策略。','A017,A018,A038'),
'15':('将任务/缓存/取消/重试/崩溃恢复/名称冲突拆成可执行算法及状态机。','A015,A024,A025'),
'16':('视频探测内容附优先级、HDR metadata不可证明ASS来源、探测失败回退为纯手动。','A021,A037'),
'17':('增加字体license/fsType/可再分发检查，subset失败等价回退与完整font环境hash。','A019,A034'),
'18':('补真实mkvmerge/mkvextract参数合同、轨道/附件ID、默认/强制/章节保留规则。','A023,A027'),
'19':('OCR首发禁用/后期Gate具体说明；声明文字人工校对问题与不做编辑器替代。','A028,A034'),
'20':('补实际CLI flags/exit codes/stdout/stderr合同、JSONL schema和重放示例。','A012,A030'),
'21':('逐项锁定tag+hash+SPDX、external/bundled策略、字体许可及FFmpeg/MKV GPL依赖义务。','A019,A026,A027,A033'),
'22':('建错误码registry、用户可执行处理建议、diagnostics隐私清单/敏感信息脱敏。','A025,A030'),
'23':('renderer-host进程隔离不能视为sandbox；拓展恶意DLL/字体/MKV攻击面安全策略。','A009,A019'),
'24':('定义golden fixture来源与许可、每组测试量化oracle/容差、自动化覆盖统计。','A010,A034'),
'25':('改单行用例为可执行test spec（fixture/steps/assertions/expected/环境/owner）。','A010,A036'),
'26':('建立M0~M4发版门槛和阶段交付物；单独标识模块实验性/strict后置。','A004,A028'),
'27':('补遗漏节点，修 P011依赖方向，添加需求到WBS到验收映射，CI检查 DAG无环。','A004,A036'),
'28':('添加Windows支持基线、安装器来源/licence、portable权限、MSVC x86 helper/CI release验证。','A013,A026,A027'),
'29':('将候选库从简介改到审计表：commit、许可证、API字段、测试状态/是否可复用。','A026,A031'),
'30':('所有网上链接对齐release/tag/commit，记录访问日/核对字段/是否定案；不要引用漂移master。','A026,A031'),
'31':('保留历史变更，但增加open issues索引并标注并非2026-10-08全审结果。','A011,A036'),
'32':('ADR每条补 alternatives/consequences/dependencies/date/status和驳回条件；Windows10策略ADR。','A013,A028,A033'),
'33':('严格区分输入ASS PlayRes、完整输出canvas、SRC FPS关联视频、Strict vs checked等术语。','A005,A008,A037'),
'34':('将V3变更日志与V3.1/审计补丁的版本流水对应，避免“最终”误导。','A011,A036'),
'35':('页面状态补并行任务、进度/部分成功、值覆盖、撤销与失焦校验；键盘可访问性。','A015,A025,A029'),
'36':('TOML示例经真实解析器校验；字段值枚举、迁移/冲突/renderer capability约束。','A015,A020'),
'37':('缓存键补全部render影响项；给出RAM/IO/磁盘预算、限流阈值和进度上报约定。','A001,A024'),
'38':('明确目录式BDN发布事务、跨卷、旧文件被占、杀进程/取消、resume版本升级。','A016,A025'),
'39':('检查表每项链接到WBS/test/owner，标记blocked项与具体过门标准。','A010,A036'),
'40':('源码审计补commit SHA/raw文件hash/可复现实验；明确没有运行编译和动态测试。','A026,A031'),
'41':('变更日志标每条受影响文件、错误级别及验证状态；审计结论仍待开发实测。','A031,A036')}
(out/'issues.json').write_text(json.dumps([dict(id=i,severity=s,topic=t,refs=r,problem=p,fix=f,gate=g) for i,s,t,r,p,f,g in issues],ensure_ascii=False,indent=2),encoding='utf-8')
counts={sev:sum(i[1]==sev for i in issues) for sev in ['P0','P1','P2']}
lines=sum(len(f.read_text(encoding='utf-8').splitlines()) for f in files)
intro=f'''# SubtitleToolkit-RS V3.1 全文档静态审计结论\n\n审计日期：2026-10-08。审计对象：V3.1 Source-Audited 基线 `{src.name}/`，共 **{len(files)} 份 Markdown，{lines} 行**。问题分级：P0（开工/产品正确性阻断）**{counts['P0']}** 项，P1（正式测试/发布前必须闭环）**{counts['P1']}** 项，P2（可放在后续迭代）**{counts['P2']}** 项。\n\n**性质声明**：这是对已有开发文档的逐文件静态一致性/完整性审计，并对关键公开规范和开源仓库做联网交叉验证；**没有运行待开发产品、没有完成Windows真实渲染器/蓝光实机测试、没有将所有第三方仓库固定到commit复验**。本审计不能替代运行时产品质量认证。\n\n## 可保留的总体方案\n\n- Windows-only；Rust桌面 GUI；手动参数优先、媒体探测辅助；不做字幕编辑器。\n- 正确区分 `ASS AST`、`Text IR`、`PGS DisplaySet IR`；保留未知扩展。\n- ASS原始PlayRes与完整16:9目标画布分层；高级Rewrite而非强制改写。\n- 外部特效DLL放renderer-host，CSRI与legacy接口探测分开。\n- PGS Basic、Advanced、Strict分级，不能把能生成SUP当大金认证。\n- 不默认接入未经许可证审计的GPL/AGPL组件。\n\n## 审计结论及最重要决策\n\n1. **文档能指导预研，却还不能无歧义指导生产开发。** 12份文档不足40行，诸如HDR(37行)、时间/FPS(37行)、批处理(36行)、MKV(20行)、CLI(24行)需要算法和失败分支规格。\n2. **先闭环 P0，再并行GUI和纯Rust核心。** P0并非代表已发现现成程序Bug，而是设计实现中高概率产生返工、错误输出或失真。\n3. **一次输入多输出必须有 RenderGroupKey。** 同组共享采样及像素帧，不同分辨率/颜色/渲染器仅共享AST/可证明等价的中间层。\n4. **“203 nit”是HDR参考白概念，不是ASS可保证的亮度。** 映射算法/播放器显示链必须写清。\n5. **Strict报告应展示“本工具的验证级别”，而不是未经认证的“所有蓝光机保证兼容”。**\n6. **开发顺序依赖须重新排 DAG。** 验证器先可独立开发，Advanced Author 依赖验证器，而非反过来。\n\n## 开发可开工/不可开工的分界\n\n现在可以启动：Windows Iced file-drop PoC、RationalTime、ASS AST bake-off、libass memory-input wrapper、FFprobe/MKV adapters PoC、PGS parser corpus 收集。\n\n暂不应声称已达到生产完成：HDR nit 映射、Strict Blu-ray、跨Profile共享渲染缓存、第三方特效DLL通用自动兼容、PGS“完全无损”、字体subset可公开分发。\n\n详情：`01_ISSUES.md`（逐项修正）、`02_ALL_DOCUMENTS.md`（42份逐文件结论）、`03_IMPLEMENTATION_CONTRACTS.md`（关键接口/算法）、`04_WBS_REBUILD.md`（修订开发顺序）、`05_VERIFIED_SOURCES.md`（联网资料与核实边界）、`06_REMEDIATION_GATES.md`（审计整改过门清单）。\n'''
(out/'00_SUMMARY.md').write_text(intro,encoding='utf-8')
lines_issue=['# 问题台账：带位置、原因、修正方案、验证条件','', '约定：位置 `09:67-75` 表示 `09_*.md` 第67~75行。P0=正确性/架构阻断；P1=生产化缺口；P2=后续质量完善。没有已经编译验证的软件缺陷结论。','']
for sev in ['P0','P1','P2']:
 lines_issue += [f'## {sev} — {counts[sev]}项','']
 for i,s,t,r,p,f,g in issues:
  if sev!=s:continue
  lines_issue += [f'### {i} {t}',f'- **证据位置**：`{r}`',f'- **问题**：{p}',f'- **最佳修正**：{f}',f'- **验收**：{g}','']
(out/'01_ISSUES.md').write_text('\n'.join(lines_issue),encoding='utf-8')
lines_doc=['# 42份文档逐一审查结果','', '文件完整性：检查了每份 Markdown 存在、前导编号、标题、主要要求、上下游文档关联、对应缺陷。以下状态均是 **规范待补**，并非“源码已坏”。','', '| 文件 | 原文行数 | 本轮补强要求 | 对应问题 |','|---|---:|---|---|']
for f in files:
 id=f.name[:2]; action,refs=bydoc[id]
 lines_doc.append(f'| `{f.name}` | {len(f.read_text(encoding="utf-8").splitlines())} | {action} | {refs} |')
lines_doc += ['','## 目录与交叉引用审计','', '- 42个连续编号：00–41；**没有文件编号缺号和重复编号**。','- 文档顺序在叙述上大体合理，但35–41属于后追加的重要产品规范，不应只作为最后附录；建议新增“按主题阅读顺序”，保留现有文件名以避免已建立的引用路径失效。','- 当前大多数文档没有明确双向链接，也没有“需求 → UI → 算法 → WBS → 测试”正式追踪矩阵。','- 不能用总纲中“源码审计后的最终版”等称谓代替可执行证据；每份文件应有状态/版本/上游规范日期。']
(out/'02_ALL_DOCUMENTS.md').write_text('\n'.join(lines_doc),encoding='utf-8')
manifest={'audit_date':'2026-10-08','baseline_dir':src.name,'files':[],'issue_counts':counts}
for f in files:
 b=f.read_bytes(); manifest['files'].append({'name':f.name,'lines':len(b.decode('utf-8').splitlines()),'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()})
(out/'BASELINE_SHA256.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
print('ISSUES',len(issues),counts,'FILES',len(files),'DOC LENGTH',lines)
