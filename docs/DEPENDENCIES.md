# 依赖与官方依据

开发前读取官方文档、公开源码及下载到 Cargo registry 的真实 API；Cargo.lock 固定解析结果。

- [GPUI Kit 0.8.0](https://github.com/longbridge/gpui-kit) / [安装要求](https://gpui-kit.com/docs/installation/) / [启动与 assets API](https://gpui-kit.com/docs/getting-started/)：Apache-2.0；固定 gpui-pre 0.3.9。
- [gpui-tray](https://github.com/domenkozar/gpui-tray)：0.3.0，Apache-2.0；关闭默认 upstream backend，启用 gpui-kit + menu-state；依赖树验证同一 GPUI snapshot。
- [global-hotkey](https://github.com/tauri-apps/global-hotkey)：0.9.0；主线程 / 原生事件循环注册，退出注销；默认只有 F1、F3。
- [XCap](https://github.com/nashaofu/xcap)：0.9.8；CoreGraphics / Windows 原生截图。按实际源码区分 macOS 逻辑 capture_region 单位和 Windows 物理单位。
- dispatch2 0.3.1（Zlib / Apache-2.0 / MIT 可选许可）：读取 registry 实际 DispatchQueue / MainThreadBound API，用于异步原生窗口移动。测试仅启用 Kit 对应的 gpui-pre 0.3.9 test-support。
- [objc2](https://github.com/madsmtm/objc2)：0.6 系列与 0.3.2 Apple Framework bindings；公开 NSWindow、NSResponder、CoreGraphics、Vision API。
- [windows-rs](https://github.com/microsoft/windows-rs)：0.62；原生窗口属性、消息、WinRT SoftwareBitmap / Windows.Media.Ocr。
- 图像 / 编码：image 0.25、tiny-skia 0.11、fontdue 0.9.5、gif 0.14；配置 serde / TOML；日志 tracing；具体解析版本和许可正文收录在打包生成的 `THIRD_PARTY_NOTICES.txt`。
- [GitHub runner 官方表](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)：macos-15 对应 ARM64，任务中另行断言 uname / Rust host，避免误用 Intel runner。

Snipaste 行为参考 [公开入门文档](https://github.com/Snipaste/feedback/wiki/Getting-Started) 与 [FAQ](https://www.snipaste.com/faq.html)。仅确认公开说明的截图、缩放、透明度、隐藏和穿透行为；未对专有二进制做逆向，未复制代码、商标、图标或图片资源。新的图标、数据模型、窗口适配均独立实现；不宣称与 Snipaste 所有细节完全一致。

风险事实与推测分开：实际观察到的 KVO 堆栈与 [GPUI Kit #3192](https://github.com/longbridge/gpui-kit/issues/3192) 相同；公开 Touch Bar provider 单独使用仍会复现；目前以空原生窗口隐藏复用缓解，短时回归通过，尚无长期验证证明根因消除。Windows 实机状态见 PLATFORM_SUPPORT，不从交叉编译推断运行可靠性。

0.1.1 新增 `objc2-screen-capture-kit =0.3.2` 与 `block2 0.6`，与现有 objc2 0.6 / Framework 0.3.2 一致；已读取 Cargo registry 的实际方法签名、错误码和许可（Zlib / Apache-2.0 / MIT）。仅用于授权验证，无另一套 GUI。官方依据：[SCShareableContent](https://developer.apple.com/documentation/screencapturekit/scshareablecontent/getexcludingdesktopwindows(_:onscreenwindowsonly:completionhandler:))、[Apple 代码签名身份 / Designated Requirement](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)。打包前重新生成目标平台的许可证通知。

0.0.1 新增 reqwest =0.13.5（MIT / Apache-2.0），启用 blocking / json / native-tls / system-proxy，已读取实际 Cargo feature 和 Proxy::all/basic_auth/no_proxy API；semver 1、sha2 0.10、tempfile 3。下载与版本元数据遵循 GitHub 官方 Releases REST API。
