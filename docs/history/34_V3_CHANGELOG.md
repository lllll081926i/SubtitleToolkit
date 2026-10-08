# V3 规划变更日志

## 核心变化
1. 平台收敛为 Windows 10/11。
2. GUI 从 Slint-first 改为 Iced 0.14。
3. Smart Adapt 从主入口降为辅助；Manual-first。
4. 新增完整 16:9 特效字幕画布硬规则。
5. 新增一输入多输出 Profile/DAG，共享渲染缓存。
6. 明确同时生成 1080p/2160p ASS/BDN/SUP 的规则与限制。
7. 特效滤镜优先 CSRI/VSFilterMod，VapourSynth/AviSynth 为 legacy fallback。
8. 移除 libimagequant 默认依赖建议。
9. 严格区分 2160p custom output 与 Blu-ray/UHD-BD PGS 合规。
10. 安装发布改为 Inno Setup + portable ZIP，增加签名/SmartScreen 策略。
11. 全部开发阶段/WBS/验收围绕 Windows-only 重排。
