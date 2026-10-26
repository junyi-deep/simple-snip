# 实际测试记录

日期：2026-10-25。设备为 Apple Silicon macOS 27.0.1，Rust 1.99.0 / Cargo 1.99.0，Xcode 27.0。以下区分模型测试、原生 API、真实桌面交互和未验证项目。

## 本机执行

| 检查 | 实际结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace` | 24 个核心测试 + 7 个 GUI 状态 / 输入测试通过；3 个依赖系统桌面的测试默认 ignored，未计为通过 |
| `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --locked --release --target aarch64-apple-darwin` | 通过 |
| `cargo check -p snip-platform --target x86_64-pc-windows-msvc` | 通过，只验证平台模块类型检查 |
| `cargo clippy -p snip-platform --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | 通过，只验证平台模块 |
| 完整 Windows GUI 交叉 `cargo check --target x86_64-pc-windows-msvc --workspace` | 失败于 GPUI 资源编译：`NotAttempted("llvm-rc")`，没有 Windows SDK 原生链接 / 运行验收 |
| `cargo tree -i gpui-pre --depth 2` | Kit / tray 共用 gpui-pre 0.3.9，无第二套 GPUI |

第三方 `block 0.1.6` 存在 future-incompatibility 提示，未冒充为本项目 Clippy 警告，也未修改上游源码规避检查。

## 原生测试（实际另行显式执行）

`cargo test -p snip-platform --test native -- --ignored --nocapture` 曾实际执行：

- CoreGraphics 连续三次抓取成功：2880×1800 物理像素，对应 1440×900 逻辑坐标。
- Apple Vision 对程序生成的本地中英文测试图识别为 `Offline 2026 测试文字`，文本和非空位置数据断言通过。测试不使用用户识别文本。

新增原生区域回归单独执行：

```sh
cargo test -p snip-platform --test native native_region_retina_odd_pixel_bounds -- --ignored --nocapture
```

通过：Retina 下 x=13 / y=17、257×129 奇数像素区域，输出尺寸精确为 257×129。

原生窗口探测程序 `cargo run -- --smoke-test` 已运行并输出 SMOKE PASS：托盘创建、设置窗口创建、无边框透明置顶、50% 透明度、穿透 / 恢复、隐藏 / 显示；新版探测额外执行三轮贴图与截图窗口复用，断言 macOS 原生句柄相同、关闭贴图的 WeakEntity 失效。探测确实输出 SMOKE PASS。旧版同一事件循环开关曾有 4 条 GPUI `RefCell already borrowed` 日志；主队列异步移动后的最新探测没有该日志，见 verification/macos-window-pool-smoke.txt。此探测**不代表整个产品验收通过**。

## 核心测试覆盖

反向框选与裁剪、负坐标混合 DPI 正反映射、旋转 / 翻转坐标、Z-order 窗口吸附、标注源像素不变、撤销重做、像素滤镜边界、OCR 跨行反向选择、原子配置保存与损坏恢复、默认仅 F1/F3、配置冲突 / 资源限制、重叠拼接与去重、失败保留、GIF 时间戳、真实 GIF 编码 / 解码、重复帧时长、历史保留不删除导出文件、工具栏下方 / 内侧定位、负坐标屏幕小图定位、拖尾缓存上限与静止自然停止。

## 实际桌面观察

使用原生桌面自动化实际打开 `.app`：设置窗口和中文内容可显示；未经授权时截图被阻止并给出系统设置说明。用户随后手动授权屏幕录制。

旧交互版本中实际观察到：冻结屏幕 / 遮罩 / 像素放大镜、框选后进入图像视图、剪贴板生成独立贴图、旋转、滚轮缩放与空格切换编辑。其间实际出现一次 AppKit Touch Bar KVO 崩溃，已记录并加入公开 API 缓解措施；之后一次截图到编辑的短时复测成功，不能推断长期修复。

用户随后要求取消独立编辑框，改为截图 / 贴图原位标注和右下浮动工具条。用户解锁后真实执行：

- 640×400 合成剪贴板图创建独立贴图；空格原位编辑，Tab 聚焦工具浮窗，选择矩形后回到原图绘制。
- 目视确认矩形出现；Command+Z 撤销消失、Command+Shift+Z 重做恢复；空格完成保留标注，再次编辑、按钮取消恢复原贴图。
- 截图选区内绘制箭头；确认工具条右对齐紧贴下方。
- 底部选区暴露原点未更新导致工具条出屏，修复为显式逻辑 Bounds + 延迟布局同步；复测靠底边选区，工具条实际位于图片下边框内侧，展开颜色 / 线宽后仍可见。
- 复制带箭头的截图，Overlay 退出；从“剪贴板贴图”入口新建，箭头保留；原位编辑 Tab → Esc 后回到原贴图，第二次 Esc 关闭，进程继续运行。
- 取消工具窗、复制退出 Overlay 都曾复现 KVO 崩溃；改为空原生窗口池后，上述结束链路短时通过。未声称长期根治。

全局 F1/F3 注册成功；自动化定向 F1 未观察到触发，物理按键 / Fn 仍需人工确认，实际链路使用应用入口。macOS 27 的全屏 Overlay 约 30 逻辑像素白带，已定位为 titled 窗口的系统约束；全屏 Overlay 的 borderless + 异步重新定位复测中白带消失。未完成全部工具组合、深色主题、小图、多屏与 Windows 回归。代码复核发现的滚动取消、OCR 选区越界、剪贴板错误反馈、工具控件可访问性四项问题已修正，并仅在源码范围复核通过。

## 可复现人工验收

1. 在已解锁桌面启动应用；检查菜单栏 / 托盘，关闭偏好设置后进程继续运行。
2. 按 F1（macOS 必要时 Fn/Globe）；拖选图像，检查冻结背景、遮罩、放大镜和像素信息。Esc 取消并重复 10 次。检查按键确实触发本应用而非系统功能。
3. 再次框选，直接在同一截图画面选择矩形 / 箭头等工具，绘制、撤销、重做。工具栏贴近选区右下方；拖到屏幕底部的选区应将工具条放在下边框内侧。
4. 复制图片，按 F3，确认新建的是独立置顶贴图；创建 3 张，分别缩放、旋转、翻转和关闭，检查资源释放。
5. 贴图按空格，确认图片没有迁移到新编辑窗口，只有工具浮条。绘制文字 / 模糊 / 马赛克，完成后再次进入编辑，原标注应可选择修改。测试小图、跨屏、边缘位置与透明背景。
6. 设置贴图半透明，T 启用穿透，检查下层应用能点击；从托盘恢复交互，隐藏 / 显示全部贴图。检查多次切换、失焦和 Spaces。
7. 在图上 OCR，拖选跨行文字复制，验证缩放 / 旋转 / 镜像后的框。重新识别与修正旧选区应无崩溃。无语言包时应清晰报错。
8. 对稳定文档长截图：手动逐帧、监测手动滚动、自动滚动 / 暂停 / 取消；导出已有结果。固定页头避开选区。取消后不得继续注入滚动。
9. 选择 GIF 区域，启动后控制栏隐藏；从托盘暂停、继续、停止，验证文件循环 / 延迟、大小、空录制、磁盘写失败与取消清理。
10. 开启拖尾并移动 / 静止，检查穿透、淡出；截图时隐藏。退出后窗口和任务不残留。
11. 两系统分别执行混合 DPI / Retina、多显示器负原点、热插拔、睡眠、权限撤销、全屏应用、剪贴板空 / 异常文件、保存失败和热键占用回归。Windows 必须在原生 Windows 10/11 AMD64 桌面运行，不能用交叉检查替代。

## 本地测试制品

已实际生成 `.app`、便携 ZIP 与 DMG；`file` 确认 ARM64，`vtool` 确认 minOS 15.0，`plutil -lint` 与 ad-hoc `codesign --verify --deep --strict` 通过。DMG 已实际执行校验。制品标注 local-test，不含正式 Developer ID / 公证声明。最新源码的 Release 窗口属性探测也实际再次输出 SMOKE PASS；这仍不替代完整产品和双平台验收。

## 后续回归（2026-10-25 晚）

- 新增并实际通过 2 项 headless GPUI 状态测试：在绝对位置 (450,650)、显示尺寸 620×215 的真实嵌入布局中映射到 1240×430 原图；切换 OCR / 标注取消旧 OCR，但保持导出会话有效，关闭窗口后使导出会话失效。
- 使用与 Kit 相同的 `gpui-pre =0.3.9` 的 dev-only `test-support`，未引入第二套 GPUI。尝试 Kit 的 `test-support` 聚合 feature 时 Cargo 解析到可选 gpui-fast / cocoa 版本冲突，改用当前后端的测试支持，未为测试降级生产依赖。
- 桌面实际验证修正后的全屏覆盖、矩形 / 箭头、带标注复制并创建贴图；复制不再停留在“正在合成”。颜色控件改为真实色块，白色也有边界。
- 后续贴图工具浮窗回归遇到外部输入与焦点变化，停止测试实例。为限制风险，实际 borderless override 最终仅用于全屏 Overlay，贴图 / 工具浮窗回归 GPUI 原有行为；不声称完成全部交互矩阵。

## 0.1.1 权限 / 标注 / 拖尾反馈回归

实际执行 fmt、workspace check、Clippy（-D warnings）、workspace tests：24 个核心 + 7 个 GUI 测试通过，3 个原生桌面测试仍为默认 ignored，本批不将它们计为通过。新增测试覆盖箭头轮廓 / 三种样式 / 零长度、标号导出与命中、选中滚轮合并撤销、图片坐标文本输入提交和修改、挂载后真实键盘输入、Kit Root 透明覆盖、同一窗口事件内取消以及贝塞尔共享切线。曾发现文字初次点击不聚焦、取消时同窗口重入两项问题，修复后测试和桌面回归通过。Clippy 曾报告新代码 collapsible_if，已修正后再次通过，无压制警告。

本机 `.app` 的实际观察：

- 原生权限消息框通过 `--permission-dialog-test` 注入拒绝事件显示；点击「打开系统设置」到正确录屏隐私页面，Simple Snip 开关为 on。未修改 / 撤销用户权限，不将事件注入称为真实 OS 拒绝测试。
- 新 debug 和 Release 包均成功进入完整截图 Overlay；Release 调试日志明确为 legacy preflight granted，因此本机成功采集不能证明 SCK fallback 路径已实测。
- 连续标号 1/2、头宽尾细红箭头、选择框与滚轮调粗细、中文原位文字实际合成到独立贴图。随后下一帧焦点修复版本中，点击图片即输入「焦点测试成功」，回车提交显示，无需第二次点输入框。
- Release 中点击取消标注后，Overlay 确实退出，回到偏好设置。原生窗口复用 / 资源释放 smoke 再次输出 SMOKE PASS。
- 用户实际移动物理鼠标，明确回复「拖尾正常」，确认全屏透明显示。用户另外反馈明显采样圆点与淡出过长；移除重复端点圆形并将默认 / 本机临时设置改为 300ms。最后这两项动态效果需物理鼠标复查。
- CUA 仅能抓取单个应用窗口，并将透明截图转为无 alpha 的 JPEG；空白特效窗口显示白底不能据此断言桌面合成白屏。定向事件不会可靠移动全局鼠标，动态拖尾由用户观察区分。

第三方许可证通知按 macOS 目标重新生成，共 515 个解析包。没有有效代码签名身份；依然是 ad-hoc 本地测试包，正式签名 / 公证未完成。Windows 平台库 check 通过，完整 GUI 和真实交互仍未验收。

新增人工步骤：无权限时截图应显示系统消息框，取消后可继续操作；打开设置按钮必须到录屏页面；本地包升级后检查当前应用身份。文字工具点击图片直接输入并回车、Esc 取消新输入、点击旧文字修改；连续编号、三种箭头，选中后滚轮 / 撤销。马赛克 / 模糊 / 橡皮的圆形范围应随粗细变化且不导出。拖尾开启后在其他应用移动鼠标，应无白底、无采样亮点，停止后约 300ms 淡出；截图时拖尾自动隐藏。

最终 0.1.1 ARM64 Release 再次构建通过；`.app` 的严格 ad-hoc 签名、Info.plist 和 DMG 校验通过。当前制品 `SimpleSnip_0.1.1_macos-arm64_local-test.zip` / `.dmg`，校验和在 `dist/SHA256SUMS`。最终原生拖尾属性日志回读为 opaque=false、background_alpha=0.0001、click_through=true；Kit Root 背景另有自动测试。各项日志见 `verification/*0.1.1*`。

## 0.0.1 发布准备

实际运行新增 4 项 updater 测试，涵盖 SemVer 新旧 / 预发布过滤、默认系统 / 直连 / 自定义代理构造、真实 loopback HTTP CONNECT 和 Proxy-Authorization（虚构测试凭据、不发往源站）、校验清单 / 平台资产 / 初始下载地址限制。workspace 总计 35 项通过。Clippy -D warnings 通过。连续带几何测试验证点上限、端点、单位法线和时间单调；新增字段兼容既有配置。

原始机器日志不提交到公开仓库，仅保留本地；此文档提供可复现步骤。正式签名与系统全交互矩阵仍不宣称完成。版本检查只有用户主动触发才联网，代理密码不写配置。

本机 0.0.1 `.app` 实际打开更新页面，系统代理检查返回当前无新版本；HTTP 模式显示可选用户名和 secure text field，空代理地址点击检查显示格式错误；恢复系统代理后可重新检查。35 项测试通过，macOS ARM64 Release 构建通过。连续渐变仍需用户物理鼠标目视确认。

## 0.0.2 实际回归（2026-10-26）

40 项本机测试通过（macOS 条件键映射测试 1 项，Windows 对应不执行；其余平台无关 39 项）；fmt / workspace check / Clippy 通过。新增选区反向边界 / 最小 1px、保留撤销坐标、旧配置默认蓝边 / 错误颜色和宽度验证、真实 GPUI 鼠标按下 / 移动 / 松开拖动手柄和 Shift+Right 调宽。Native smoke 实际执行三次「有置顶贴图 → 隐藏设置 → 重开设置」，断言设置窗口可见，其他透明 / 穿透 / 窗口复用断言通过。热键成功注册，但物理 F1 / Fn 仍待解锁桌面人工验证。

首次公开 CI：macOS ARM64 和 Windows AMD64 均完成 check / Clippy / test / Release；Windows 许可证输出 cp1252 错误已修复为 UTF-8，后续 v0.0.1 标签流水线成功发布。编译运行测试和实际桌面交互仍分开记录。

人工补充：创建贴图并关闭设置，从托盘重新打开，确认设置位于贴图前方；切到其他应用按真实 F1 / 必要时 Fn+F1，检查日志 global hotkey received 和 Overlay。配置不同颜色 / 粗细 / 阴影保存，现有贴图立即变化，复制内容不含外框。截图绘制后拖动角和边改变宽高，确认标注保持位置、撤销正常，工具条末尾紧靠图片右边。

0.0.2 最终发布确认：GitHub Actions 40251248659 的 macOS ARM64、Windows AMD64 和 Release job 全部成功。远程日志实际为 macOS 9 GUI/键映射 + 27 core + 4 updater = 40；Windows 8 GUI + 27 core + 4 updater = 39，均 0 失败；3 项依赖桌面的 native 测试仍 ignored。三份制品的 GitHub digest 与 SHA256SUMS 一致。实际下载 Windows ZIP 并校验哈希，读取 PE 确认为 AMD64 / GUI subsystem；依赖 VCRUNTIME140.dll，运行库要求已补充到文档和发布说明。更新模块实际读取最新 v0.0.2、下载 DMG 并通过 SHA256。最终本地 `.app` 严格签名检查、Info.plist、DMG 校验及原生 smoke 均通过。桌面仍锁屏，未将 F1/Fn 真实输入和最终目视操作标记通过。
