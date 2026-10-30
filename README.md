# Simple Snip

Rust + GPUI Kit 的离线桌面截图工具，仅面向 **Windows AMD64** 和 **macOS ARM64**。
当前版本为 **0.0.8**（首个公开版本为 0.0.1）：[GitHub](https://github.com/junyi-deep/simple-snip) · [下载](https://github.com/junyi-deep/simple-snip/releases/latest)。
当前是开发版本，完整产品验收状态见 [进度](docs/PROGRESS.md)，不应视为已经完成全部需求的正式发行版。

## 运行

需要 Rust 1.99+。macOS 15+、Apple Silicon 与 Xcode；Windows 10 2004+ / Windows 11 AMD64、VS 2022 C++ Build Tools、Windows SDK 和 CMake。

```sh
cargo run
```

默认仅注册 F1（区域截图）与 F3（剪贴板贴图）。关闭设置窗口后应用继续驻留系统托盘 / macOS 菜单栏，从菜单退出。重复启动会被应用锁阻止。

- 截图：拖动框选 / 单击吸附窗口；生成选区后可拖动八个手柄调整宽高；画布聚焦时 Shift + 方向键按像素调整大小；A 当前显示器全屏，C 复制像素 HEX，框选结束即在原选区标注，Esc 取消。
- 编辑：图片右下方的浮动图标工具条选择标注工具，在原图上绘制。工具栏按内容收紧宽度；工具、形状、颜色、箭头样式和各工具尺寸自动保存，下次进入编辑及重启后恢复。文字工具点图像后原位输入，回车提交；Esc 从画布、文字 / 数值输入框及工具栏取消整次编辑；再次点击已有文字可修改。字号 / 线宽支持输入、加减与滚轮；矩形工具提供圆角 / 椭圆 / 菱形 / 三角形，线条提供折线（点击添加节点、双击或 Enter 完成）/ 曲线，马赛克按下即涂抹。支持连续标号、三种箭头样式、选择移动、选中后滚轮调粗细、颜色、撤销重做、原生另存为（记住目录）、PNG/JPEG 导出、复制与贴图。
- 贴图：截图工具栏固定图片会保留选区在桌面上的位置和显示尺寸。单击选中后显示蓝色边框，未选中时无边框。Windows 选中贴图后，鼠标移出图片范围仍可滚轮缩放；Command/Ctrl + 滚轮调整透明度；1 旋转，2/3 翻转，T 穿透，空格在原贴图内编辑；编辑时 Tab 聚焦工具条、Esc 取消、空格完成。选中后 Esc 关闭，亦可用 Command+W / Ctrl+W 或右键「关闭贴图」。穿透后从托盘「恢复全部贴图交互」恢复。「贴图」设置可修改选中边框颜色、粗细和阴影，保存后立即应用。自 v0.0.3 起采用选中时显示边框的行为。
- OCR：在浮动工具栏点击 OCR 图标，再在文字结果区域按字符拖选，Ctrl/Cmd+C 复制，支持跨行复制、全部复制、基础 TSV 与选中行修正。
- 长截图 / GIF：F1 框选后，通过工具栏第一个「截图 ▾」切换长截图 / GIF，也可在画布按 L / G；控制栏提供帧率 / 采集间隔输入和数字快捷键。仍可从托盘启动，控制栏紧贴区域右下侧，空间不足时移到区域内侧并从采集中排除。长截图自动监测左 / 右 / 上 / 下滚动；请小幅滚动稳定内容，可暂停、继续、自动向下滚动和完成编辑。GIF 在同一控制栏开始、暂停 / 继续、停止保存和取消。
- 鼠标拖尾：默认开启，可从托盘 / 设置关闭。连续颜色 / 宽度渐变，默认 300ms；生命周期可设 100–3000ms，保存立即生效。Windows 截图时通过系统排除采集保留拖尾显示；长截图 / GIF 录制时隐藏。保留用户已保存的开关选择。
- 更新：偏好设置 → 关于、更新与权限 → 检查更新。支持系统代理（默认）、无代理、自定义 HTTP 代理；可选用户名和本次运行密码。点击「升级并重启」后下载并校验 SHA256，准备同目录安装文件，退出当前应用后自动替换并重启；替换或启动失败时尝试恢复旧版本。安装目录需可写；macOS 需从 .app 运行。0.0.6 及更早版本仍需手动安装一次新版。
- 设置：调整 F1/F3 或留空禁用；其他可选全局快捷键初始为空。点击保存设置，热键动态注册，配置保存在用户目录。

macOS 可能需要 Fn/Globe + F1/F3；也可改用其他组合键。屏幕录制被系统拒绝时弹出原生消息框，点击「打开系统设置」进入录屏权限页。若本地测试包更新后开关已开仍失败，请移除旧 Simple Snip 条目，再添加当前 `Simple Snip.app` 并重启；临时 ad-hoc 签名随构建变化，正式 Developer ID 签名尚未提供。辅助功能权限仅自动滚动需要，不绕过系统限制。

## 检查

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
# 原生测试（真实桌面 / 系统语言包）
cargo test -p snip-platform --test native -- --ignored --nocapture
# 原生窗口属性与生命周期探测，不等同于视觉验收
cargo run -- --smoke-test
```

[架构](docs/ARCHITECTURE.md) · [测试](docs/TESTING.md) · [平台支持](docs/PLATFORM_SUPPORT.md) · [已知问题](docs/KNOWN_ISSUES.md) · [构建](docs/BUILD.md)

推送与 Cargo / Info.plist 版本一致的 `vMAJOR.MINOR.PATCH` 标签后，双平台 GitHub Actions 自动构建并创建 Release。制品明确标记 unsigned；签名 / 公证和完整桌面验收状态见文档。

Windows 下载运行需要 [微软官方 x64 Visual C++ v14 运行库](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)。便携 ZIP 不含该运行库；开发构建工具要求与运行要求不同。
