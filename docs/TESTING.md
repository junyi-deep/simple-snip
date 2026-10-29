# 实际测试记录

## 2026-10-30：v0.0.6 发布复核

已推送注释标签 `v0.0.6`，目标提交 `6666f571f376a986fa0a6e42d8ff220bcd3b1d00`。[流水线 40913442851](https://github.com/junyi-deep/simple-snip/actions/runs/40913442851) 的 Windows、macOS、Release job 全部成功；实际日志为 Windows 54 项、macOS 53 项自动测试通过，两平台各另有 3 项系统测试默认 ignored。两个原生目标均通过 fmt / check / Clippy / Release / 打包和资产上传。

[v0.0.6 Release](https://github.com/junyi-deep/simple-snip/releases/tag/v0.0.6) 已发布，非草稿、非预发布。已实际下载三个公开安装包及 SHA256SUMS，哈希同时匹配校验文件和 GitHub digest；两个 ZIP 完整性、Windows AMD64 / PE32+ / GUI subsystem、macOS ARM64 Mach-O 与 Info.plist 双版本 0.0.6 均通过。未运行下载的发布二进制；本机原生 smoke 使用源码 Debug 构建，未在 Windows 上执行 DMG 挂载或 macOS 签名验证。完整目视 / 混合 DPI 矩阵仍不计为通过。

## 2026-10-30：v0.0.6 功能与性能回归

Windows 本机最终执行 54 项自动测试通过（18 GUI/颜色转换 + 29 core + 3 Windows 原生属性 + 4 updater），另 3 项系统测试默认 ignored。fmt、Clippy -D warnings、版本一致性校验通过。新增普通设置窗口 APPWINDOW / 非 TOOLWINDOW / 非 TOPMOST 原生属性回归。

修复设置窗口激活观察者重新置顶，以及所有窗口样式强制 TOOLWINDOW 的问题；设置创建 / 重开均为非置顶 APPWINDOW。移除贴图双击隐藏。文字输入使用 appearance(false) / bordered(false) 透明原位编辑，保留一层蓝色边框和回车 / Esc；修改旧文字时临时渲染不含旧文字的文档，取消恢复。标号显示下一序号，支持加减并自动递增，新增 6、7、调整到 3 后下一个为 4 的事件回归。长截图 / GIF 控制栏期间显示穿透区域边框，关闭会清理；选区工具栏改用统一右下角避让布局。

Windows 截图窗口按显示器复用，Root 替换释放旧文档；取消事件测试改为检查 Root 已更换。大图颜色转换最多使用 4 个后台线程，新增奇数尺寸 / 分块边界 / alpha / 源图不变回归。Debug 原生探测实际输出 SMOKE PASS：三轮完整采集 / 取消 / 拖尾可见，贴图位置及客户区尺寸、设置非置顶、透明度和穿透恢复、文档释放、控制栏生命周期通过。分段日志：采集约 280–463ms，颜色转换约 95–129ms，首次 Overlay 根节点创建 282ms，复用 0–1ms；完整采集 / 呈现 / 取消约 793–868ms，含人为呈现等待。不是受控性能对比或 Release 时延承诺。

## 2026-10-30：Windows 窗口交互回归修复（工作区）

设置窗口创建及重开均使用非置顶样式；拖尾在鼠标所在显示器检查并恢复 Z-order，覆盖设置 / 任务栏后抬高且不激活窗口。Windows 成功设置 WDA_EXCLUDEFROMCAPTURE 后，截图 / 录制时保持拖尾可见；系统拒绝排除时保留暂停回退，避免污染导出图片。贴图改用原生客户区拖动消息，窗口显示前移除 WS_CAPTION 等原生边框并精确定位，缩放同样使用精确尺寸。

本机 Windows workspace 51 项测试通过，另有 3 项系统测试保持 ignored；新增回归检查无原生边框的定位 / 缩放尺寸，以及截图排除成功时拖尾保持可见。fmt、Clippy（-D warnings）和 Debug 构建通过。原生 smoke 实际得到 SMOKE PASS：截图期间拖尾可见，设置窗口非置顶，贴图呈现后的原点 / 宽高与指定选区精确匹配，窗口客户区等于外部尺寸；其他透明度 / 穿透恢复 / 文档释放检查通过。

原生探测曾抓到首次激活重放 GPUI initial_placement 覆盖定位的问题；现改为在挂载图像前消费初始定位、隐藏窗口，再无边框定位和显示，复测通过。物理鼠标拖动手感、设置与其他应用切换、任务栏动态覆盖和混合 DPI 多屏的完整目视矩阵仍未计为已验收。本节是当前工作区验证，非既有 v0.0.5 发布包的结果。

## 2026-10-28：v0.0.5 发布复核

已推送 `v0.0.5` 注释标签，目标提交为 `075ce15ecc145127a8049f9b4096b47be5c0f838`。[流水线 40714940524](https://github.com/junyi-deep/simple-snip/actions/runs/40714940524) 的 Windows AMD64、macOS ARM64 和 Release job 全部成功；实际日志为 Windows 49 项、macOS 50 项测试通过，两平台各有 3 项系统测试默认 ignored。两个目标的 fmt / check / Clippy / Release / 打包 / 上传均通过。

[v0.0.5 Release](https://github.com/junyi-deep/simple-snip/releases/tag/v0.0.5) 已发布，非草稿、非预发布。已实际下载公开的三个安装包与 `SHA256SUMS`，每个安装包的 SHA-256 同时匹配校验文件和 GitHub 资产 digest；两个 ZIP 完整性、Windows AMD64 / PE32+ / GUI subsystem、macOS ARM64 Mach-O 与 Info.plist 双版本 `0.0.5` 均通过。未执行下载的二进制；本机 Windows 原生探测使用本地构建，未在 Windows 上声称执行 DMG 挂载 / macOS 签名验证。

## 2026-10-28：Windows 拖尾、贴图窗口与响应优化

Windows 原生客户区坐标转换新增隐藏窗口回归，覆盖负坐标和 100% / 125% / 150% / 200% 比例；GUI 回归覆盖截图暂停时清空轨迹、恢复时不连接旧轨迹。Windows workspace 共 49 项自动测试通过，3 项系统测试默认 ignored，未计为通过。

本机实际执行 `--smoke-test` 并得到 SMOKE PASS：三轮完整桌面采集 / Overlay 呈现 / 取消 / 拖尾恢复，检查拖尾窗口保持可见、置顶和穿透；通过实际 Pin 事件创建并呈现合成图片，检查系统按钮标志；三轮贴图透明度、穿透 / 恢复、设置重开、文档释放和采集控制栏生命周期通过。测试模式使用独立实例锁和默认配置，不注册全局热键，可与已运行的用户实例并存。探测为属性和生命周期检查，不是物理鼠标动态目视验收。

截图和剪贴板贴图的 RGBA/BGRA 转换移到后台，Windows 吸附改为只枚举矩形，关闭拖尾时省略截图前等待、开启时等待从 70ms 调整为 35ms。Debug 探测中的完整采集 / 呈现 / 取消周期约 1.2–1.3 秒，包含人为呈现等待和窗口创建；不能作为 Release 性能基准。真实混合 DPI 多屏、热插拔和性能基准仍待验收，本次未执行 macOS 桌面回归。

## 2026-10-27：v0.0.4 发布复核

v0.0.4 标签为 a91da91f502e99a73765f3ea5785649831bb201c。[流水线 40528736685](https://github.com/junyi-deep/simple-snip/actions/runs/40528736685) 原生 Windows AMD64、macOS ARM64 与自动 Release job 全部成功，实际日志为 macOS 15 + 29 + 1 + 4 = 49 项、Windows 14 + 29 + 0 + 4 = 47 项，均另有 3 项系统桌面测试 ignored。fmt / check / Clippy `-D warnings` / Release / 打包与资产上传全部成功。

[Release](https://github.com/junyi-deep/simple-snip/releases/tag/v0.0.4) 于北京时间 2026-10-27 23:18:23 发布，非草稿、非预发布。已实际下载公开的 macOS ARM64 DMG / ZIP、Windows AMD64 ZIP 与 SHA256SUMS，全部哈希同时匹配校验文件和 GitHub 资产 digest；两个 ZIP 完整性、Windows AMD64 / PE32+ / GUI subsystem、macOS ARM64 Mach-O / Info.plist 双版本 0.0.4 和 DMG `hdiutil verify` 均通过。没有执行下载的发布二进制；下面的桌面验收使用本地修复测试包，签名 / 版本身份可能不同。

## 2026-10-27：v0.0.4 修复与桌面补验

用户解锁后继续桌面验证，并要求截图固定不改变位置和尺寸。实际定位到截图导出事件只传图像、PinView 总是居中并适配至 680×500 的问题；改为传递可选桌面逻辑 Bounds，截图使用当前选区，剪贴板 / 历史仍走自动放置。

`cargo test --locked --workspace` 49 项通过（15 GUI、29 core、1 原生内存像素、4 updater），另 3 项桌面测试 ignored。新增测试覆盖负原点显示器、Retina 物理 / 逻辑比例、调整后的选区及超过旧适配上限的内容尺寸，验证阴影留白在选区外侧。workspace Clippy `-D warnings` 和 ARM64 Release 构建通过。

已实际打开现有 v0.0.3 的截图 Overlay 与设置页；900×660 设置窗口的长截图说明完整换行。最初桌面工具对 Overlay 拖拽返回 windowNotFoundAtPosition，且选中了偏好设置或透明拖尾窗口。用户协助打开控制栏后，临时关闭拖尾、隐藏设置，并在新进程同步 AX 目标，后续实际完成如下补验。

- 修复版 `dist/placement-preview/Simple Snip.app`：在桌面物理坐标 (1000,500) 至 (1800,1100) 框选 800×600，实际点击「贴到桌面」。原生窗口为 x=488、top=238、424×324 逻辑像素，去掉外侧 12px 留白后内容正好为 (500,250)、400×300；目视同一内容和蓝色选中框，随后 Esc 正常关闭。截图权限实际通过，没有修改 TCC。
- 原生 smoke 新增两轮贴图目标 Bounds 检查，等待异步移动后读取实际 NSWindow frame；位置 / 尺寸断言通过，窗口复用 / 资源释放和原有控制栏断言仍通过。该属性检查与上述真实截图固定链路分别记录。
- 用户协助打开的 GIF 选区为 1728×894：实际点击开始、暂停、继续、停止保存和系统预览，控制栏持续可操作，界面显示 8.8 秒、保存成功。生成本地 `Snip_20261027_225126_588.gif`，ffprobe 为 1280×663、58 帧、8.72 秒，ffmpeg 全帧解码无错误；系统预览实际打开且可查看多帧。没有据此宣称 15 FPS 达标，也未将本地屏幕内容提交或上传。
- 上述 GIF 控制栏位于选区外侧；后续另行完成内侧排除测试。初始长截图尝试读到前台 Codex，不能代替稳定页面验证；用户把离线网格页切到真实前台后才执行下面的拼接验收。

用户随后把离线网格页实际切到前台。长截图已补验：框选 (500,400) 至 (1700,1300) 的 1200×900 物理像素区域，控制栏显示首帧 1 段；向下滚动后为 1200×1232 / 2 段，向上回滚不重复增大；通过浏览器水平滚动条向右移动后为 1593×1232 / 3 段，向左回滚仍保持尺寸和段数。暂停 / 继续检测可用，实际点击「完成 / 编辑」进入原位标注并用 Command+S 导出 PNG。PNG 实测 1593×1232 RGBA，网格行列连续；未访问的 393×332 右下区域恰为 130476 个透明像素，其余像素不透明。混合方向缺口策略符合已记录限制。

自动滚动实测被 macOS 辅助功能权限拦截，界面明确提示系统设置路径及手动滚动替代方式，已有 3 段结果保留并可正常完成导出。没有更改系统权限，自动输入滚动成功路径仍未验收。

用户实际将设置窗口缩至最小：原生 frame 为 760×572，content 为 760×540。目视检查长截图、GIF、关于 / 更新与权限三页，说明完整换行，FPS 按钮、输入框及保存栏不裁切，内容仍可滚动。

另以 A 选择完整 2880×1800 显示器录制 GIF，右下控制栏位于采集区域内。录制过程中控制栏实际可见可操作，保存的 `Snip_20261027_230834_858.gif` 为 1280×800、48 帧、14.26 秒；ffmpeg 完整解码通过，提取首帧目视确认右下控制栏未进入画面。此前一次全屏录制误点取消，实际验证取消后未生成最终 GIF，随后重新开始完成保存；没有把被取消的录制当作保存通过。这里只确认本机的控制栏排除，不推断 Windows 或多显示器行为。

用户新增默认开启拖尾要求，TrailConfig 默认改为 true；现有默认配置测试同时确认开启且仍只有 F1/F3，原子保存测试补验用户主动关闭后重载仍为 false。本机 49 项测试、Clippy、ARM64 Release 再次通过。测试临时关闭的拖尾已通过设置开关恢复为 on，配置已持久化；本轮测试页已关闭，未上传屏幕图像或 GIF。

修复提交 7fd6a9d 的[双平台预检 40523548563](https://github.com/junyi-deep/simple-snip/actions/runs/40523548563) 完整成功，两个目标均完成 fmt / check / Clippy / tests / Release / 打包，Release job 因手动预检而跳过。后续原生几何探测补充不改变截图固定的运行逻辑。

## 2026-10-27：v0.0.3 标签发布与制品复核

用户要求先发布、后补桌面验证。v0.0.3 标签指向 6b4ef06ff5a6d4a2d61b9b108781ed30829f8c8e；[标签流水线 40449003057](https://github.com/junyi-deep/simple-snip/actions/runs/40449003057) 的两个原生平台和 Release job 全部成功，均完成 fmt / check / Clippy / tests / Release / 打包。公开 [Release](https://github.com/junyi-deep/simple-snip/releases/tag/v0.0.3) 于 04:23:33 UTC 发布，非草稿，GitHub latest API 已返回 v0.0.3。

实际下载公开资产到本机临时目录后验证：

- macOS ARM64 DMG、ZIP 与 Windows AMD64 ZIP 的 SHA256 均匹配 SHA256SUMS 和 GitHub 返回的资产 digest。
- 两个 ZIP 的完整性检查通过；Windows 可执行文件为 AMD64 / PE32+ / GUI subsystem；macOS Info.plist 为 0.0.3，主程序为原生 ARM64 Mach-O。
- `hdiutil verify` 确认下载的 DMG 校验有效。没有运行下载的二进制，也没有将包结构检查当成桌面交互验收。

macOS 仍为 ad-hoc，未完成 Developer ID 签名 / 公证。新图标外观、长截图 / GIF 控制栏的实际交互和采集排除、最小设置窗口目视继续待验收。下面发布前记录保留为历史，不再阻止本次发布。

## 2026-10-27：v0.0.3 发布前检查

发布安排已按用户最新要求调整为先发布、后补桌面验证。下列待验收项目仍保持未验证状态；发布不代表这些项目已通过。

用户已确认新贴图在其他应用与贴图间切换后能正常选中并关闭。此前修复的双平台工作流 [40355265508](https://github.com/junyi-deep/simple-snip/actions/runs/40355265508) 已成功完成完整编译、Clippy、测试和打包；那次运行对应 4652e57，不能代替本次新增功能的 CI。

本次代码的本机结果：fmt / Clippy `-D warnings` 通过，47 项自动测试通过（13 GUI / macOS 键映射、29 core、1 CoreGraphics 像素转换、4 updater），3 项 native 测试仍 ignored；Windows 平台模块 MSVC target 类型检查通过。新增四方向 / 回滚 / 换轴透明区域、周期性纹理拒绝、失败 / 内存上限保留已有结果测试，以及控制栏右侧 / 底部避让、窄屏几何测试。CoreGraphics 纯内存原生转换实际执行，确认 RGBA 通道和行序。ARM64 Release 与 ad-hoc 打包已成功，后续窗口生命周期变动仍需最终重建。

最终本机重建与 ad-hoc 签名验证已成功。原生 smoke 实际通过，新增控制栏的置顶 / 可交互属性与关闭后清除活动会话断言，未出现 RefCell / panic / ERROR。DMG 以只读、不打开 Finder 的方式挂载，确认根目录为完整 Simple Snip.app 后卸载。以上是窗口属性与包结构检查，不代表锁屏期间完成了视觉或视频采集。

预检工作流 [40427251004](https://github.com/junyi-deep/simple-snip/actions/runs/40427251004) 对 a0ed074 的两个目标均完成完整编译、Clippy、测试、Release 与打包，Release job 因不是标签而跳过。随后补充的自动滚动重置、回访时不误判到底、完成等待当前帧、内存预留与特效协作改动已再次通过本机 47 项测试和 Clippy；最终标签流水线仍需在桌面验收后运行。

最终源码 893d573 的预检工作流 [40429397908](https://github.com/junyi-deep/simple-snip/actions/runs/40429397908) 两个平台全部成功。实际日志为 macOS 13 + 29 + 1 + 4 = 47 项，Windows 12 + 29 + 0 + 4 = 45 项；均另有 3 项 native 桌面测试 ignored。完整 check / Clippy / Release / 包生成及上传成功，Release job 仍按条件跳过。再次调用桌面工具仍明确返回 Mac 锁屏，因此尚未创建 v0.0.3 标签或 Release；新控制栏的实际采集 / 排除和最小设置窗口目视仍待解锁。

Mac 再次锁屏，已请求用户解锁。新图标的菜单栏外观、长截图 / GIF 的控制栏点击及采集排除、设置说明在最小窗口的目视尚未通过。准备了完全离线的 `tests/fixtures/scroll-grid.html` 双轴滚动页；浏览器插件不可连接本机 Chrome，IAB 无法连接本机 localhost，拟在解锁后用原生浏览器操作。不得将这些尝试当成已完成桌面验收。

## 2026-10-27：贴图选中与关闭回归（主分支，尚未发布）

用户已实际确认 v0.0.2 中「有贴图时菜单栏打开偏好设置」和 F1 截图正常；这两项不再标为待反馈。Shift+Esc 的外部冲突来源未确认，不增加全局拦截或绑定。

- 修复贴图点击时只设置画布焦点的问题：显式激活应用 / 窗口，监听激活恢复浏览焦点并更新选中外框。默认未选中时不画边框，选中显示配置的蓝边，编辑工具条聚焦时保留所属贴图高亮。
- GPUI 输入测试实际执行：从第二张贴图切到第一张、判断第一张选中 / 第二张失焦、Esc 事件只关闭第一张、释放文档实体且保留第二张；编辑画布与独立工具条的 Command+W / Ctrl+W 均分发到所属贴图。
- `cargo fmt --all -- --check`、workspace Clippy `-D warnings` 通过；workspace 共 42 项通过（11 GUI / macOS 键映射、27 core、4 updater）；3 项原生桌面测试仍 ignored。ARM64 Release 构建成功，原生 smoke 再次通过。
- 独立本地测试包 `dist/pin-focus-preview/Simple Snip.app`：目视选中蓝边；右键菜单实际出现「关闭贴图」并成功关闭；点击聚焦后分别以 Esc、Command+W 实际关闭。跨应用真实失焦的最终目视仍需人工确认，定向窗口事件不能证明系统焦点切换。Windows Ctrl+W 的真实桌面行为未验收。
- 重启前通过原应用的保存操作保留了两张测试贴图，位于本机 Pictures/Simple Snip；未提交图像或屏幕内容。v0.0.2 应用包、标签与发布资产均未覆盖。

日志：`/tmp/simple-snip-pin-focus-{tests,workspace,clippy,build,smoke,ui}.log`。本地包是未发布、ad-hoc 测试版，不代表新的正式 Release。

## 2026-10-26：v0.0.2 解锁后桌面补验

使用现有 `dist/Simple Snip.app`（0.0.2）测试，未重建或重新签名。

- 实际捕获到系统拒绝录屏后的原生提示。系统设置的 Simple Snip 开关为 on，用户确认移除旧条目并重新添加当前应用；随后重启测试。最终调试日志为 `screen capture permission: legacy preflight granted`，并实际展示冻结桌面 Overlay。不能仅凭开关 on 推断正在运行的应用已获准，也未通过日志确定先前拒绝的全部系统原因。
- 拖动选区为 990×500 像素，绘制矩形后拖动右下手柄改为 1190×800，再改为 1240×1128；尺寸提示随之变化，既有矩形保留。
- 工具条在有空间时紧贴图片下方右侧；选区接近屏幕底部后，工具条实际转到图片底边内侧，右边沿仍对齐。
- 点击「贴到桌面」实际生成独立贴图。另从本地原创图标测试文件创建贴图，细蓝边目视正常，滚轮缩放有效。
- 自动化的定向 F1 输入未产生全局截图，不能替代物理 F1 / Fn 验收。后续人工操作期间，日志于本地 23:42:04 实际收到 `global hotkey received id=160`，随后屏幕授权通过并于 23:42:08 生成第二张贴图，证实全局事件已到达采集链路。具体物理组合键以及菜单栏重开设置的可见性仍等待人工反馈；原生可见性探测结果另见既有记录。

原始后端日志留在本机 `/tmp/simple-snip-002-unlocked.log`；不提交桌面截图和用户屏幕内容。此轮仅补交互验证记录，没有修改发布二进制。

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
