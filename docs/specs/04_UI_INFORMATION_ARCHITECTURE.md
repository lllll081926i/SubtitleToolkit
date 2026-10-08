# Windows UI 信息架构

## 1. 框架
- Rust + Iced 0.14。
- 单主窗口，多工具页面；不使用 WebView。
- Windows 原生文件选择器（建议 rfd/IFileDialog）。
- 支持 Windows Explorer 文件/文件夹拖放。
- 默认深浅色跟随系统，可在设置覆盖。

## 2. 主窗口布局

推荐 1440×900 作为设计基线，最小窗口 1100×720。

```
┌ Titlebar / command bar ──────────────────────────────────────┐
│ 左导航  │                 工作区                 │ 任务抽屉 │
│ 220px   │                                          │ 320px    │
│         │                                          │ 可收起   │
├─────────┴──────────────────────────────────────────┴─────────┤
│ 状态栏：依赖 / renderer / 队列 / 输出目录 / 更新状态          │
└──────────────────────────────────────────────────────────────┘
```

## 3. 导航顺序
1. 首页
2. 转换
3. ASS / HDR
4. 时间 / FPS
5. ASS → BDN / SUP
6. SUP 工具
7. 批量 / 多输出
8. 字体 / MKV
9. 同步助手（可选）
10. 诊断
11. 设置

Smart Assist 不是一级导航第一项；它是首页的辅助卡和各参数页的“从视频填充”。

## 4. 首页
### 主区
- “添加字幕”大按钮/拖放区。
- 最近使用 preset。
- Quick Actions：格式转换、HDR、FPS、ASS→SUP、SUP调整、多输出批量。

### 辅助区
- “读取目标视频参数”小卡片。
- “自动同步助手”小卡片。

## 5. Advanced 模式
不是独立 app 模式，而是每个工具页右上的“高级参数”展开开关；这样用户不会在两个界面体系之间迷路。

## 6. 任务抽屉
始终可从右上角打开：
- Running / Queued / Completed / Failed；
- 每任务显示 input、当前 stage、progress、ETA（有依据时）、output count；
- 可取消/重试/打开输出/查看日志；
- renderer-host crash 显示专门错误类型。

## 7. Windows 原生体验
- Ctrl+O 添加字幕；Ctrl+Shift+O 添加文件夹；Ctrl+Enter 执行；Esc 取消当前对话框。
- Explorer 拖入 ASS/SRT/SUP/视频/MKV。
- 右键文件关联后可“用 SubtitleToolkit 处理”。
- Jump List/最近文件可以后期添加，不作为 MVP 阻塞项。
