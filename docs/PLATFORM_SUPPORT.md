# 平台支持

仅发布目标：`win-amd64`（x86_64-pc-windows-msvc）和 `macos-arm64`（aarch64-apple-darwin）。其他目标在编译入口明确拒绝；不含 Linux、Intel macOS、Windows ARM / 32 位或 Universal 构建任务。依赖的 Cargo.lock 中可能存在其自身的其他平台条目，不代表应用支持那些平台。

## 最低系统与实际设备

- GPUI Kit 0.8 官方安装文档要求 macOS 15+、Windows 10+。本项目据此将 Info.plist 最低系统设为 macOS 15.0；Windows 面向 10/11 AMD64。**最低版本兼容尚需真机验证。**
- 当前实测设备：macOS 27.0.1 / Apple Silicon；Rust 1.99.0，Xcode 27.0。
- Windows 安装了 Rust MSVC target，只完成平台模块交叉类型检查；本机无 Windows GUI / SDK 测试环境。

| 能力 | macOS ARM64 | Windows AMD64 |
|---|---|---|
| 原生平台模块编译 | 已通过 | Windows 原生 CI check / Clippy 已通过 |
| GPUI GUI 编译 / 启动 | 已通过 | Windows CI 完整编译 / Release / 打包通过；本机交叉仍缺 llvm-rc，桌面启动待验 |
| 托盘 / 菜单栏 | 原生创建探测通过 | 代码已接入，未运行验收 |
| F1/F3 注册 | 真实注册成功 | 代码已接入，未运行验收 |
| F1/F3 真实物理按键 / Fn | 仍需人工确认 | 未验证 |
| 透明 / 无边框 / 置顶 / 穿透恢复 | 原生属性回读通过 | 实现与类型检查通过，未运行 |
| Retina 截图 | 2880×1800 / 1440×900 真实采集通过 | 混合 DPI 真机未验证 |
| 本地 OCR | Vision 中英文合成图测试通过 | Windows.Media.Ocr 实现已类型检查，语言包 / 实机未验证 |
| 原位标注 / 复制到贴图 / 工具条避让 | 矩形、箭头、撤销重做、完成取消与底边避让实际通过 | 未验证 |
| 全屏透明鼠标拖尾 | 用户物理鼠标观察确认正常；多屏 / 功耗未验收 | 未运行 |
| 完整 UI 回归、多显示器、Spaces / 休眠 | 其余待验收，KVO 仅短时缓解回归 | 未验证 |

## 权限

macOS 屏幕录制采用 CGPreflightScreenCaptureAccess 快速预检，未确认时用 ScreenCaptureKit SCShareableContent 验证；不再将预检 false 直接当成拒绝。系统明确拒绝时显示原生消息框，按钮进入录屏权限页；用户自行授权。应用包有独立的 TCC 身份，开发进程权限不等同于应用包权限。本地 ad-hoc 重新签名可能需要系统重新确认。

手动截图和窗口级吸附不强制要求辅助功能。自动滚动使用公开 CGEvent；缺少 Accessibility 权限时明确返回错误，用户可手动监测滚动。鼠标位置使用 CGEvent 查询，不安装键盘 / 鼠标事件 Tap。

Windows 使用 OS 剪贴板、窗口样式与消息 / 原生截图；不绕过受保护窗口或 UIPI。OCR 需要系统对应语言支持，不可用时提示安装语言包。macOS Spaces / 全屏置顶行为受系统限制，不能保证覆盖安全屏幕。

CI 仅配置 windows-2022 AMD64 和 macos-15 ARM64，并在任务中断言真实 CPU 架构。公开仓库已创建；v0.0.1 的 Windows AMD64 / macOS ARM64 原生 CI 编译、Clippy、单元测试、Release 和打包已通过，不能替代真实桌面交互。

0.1.1 桌面实测：当前应用包能抓取全屏并进入原位标注；系统设置的 Simple Snip 录屏开关为 on（只读取，未修改）。通过诊断事件实际显示原生权限消息框，点击后打开正确的「录屏与系统录音」设置页。没有撤销用户权限来模拟真实拒绝；SCK 回退是否触发应以调试日志区分。编号、头宽尾细箭头、选择框 / 滚轮、图片内中文输入与提交已在 macOS 桌面观察。Windows 仍仅平台模块类型检查，未运行 GUI。

0.0.2 macOS 更换为 Carbon dispatcher 独占热键注册；注册与原生设置可见性探测通过。F1 / Fn 实际按键和新框架视觉因锁屏待验，未假称完成。
