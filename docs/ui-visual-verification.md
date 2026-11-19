# 原生 UI 视觉验收

2026-11-19 · macOS 27.0.1 / Apple Silicon · 单块 Retina 屏幕，GPUI 以 2× 绘制。使用当前工作区 Debug 二进制和内置合成素材，未安装额外应用包。偏好设置浅色内容 900 × 660、深色最小内容 760 × 540，尺寸不含系统标题栏。下面是以真实 GPUI 窗口捕获结果为依据的检查。

| 范围 | 目视结果 | 仍需验证 |
| --- | --- | --- |
| 偏好设置 | 检查了浅色通用页和最小尺寸深色的全部 11 页；导航图标、标题、控件与状态区对齐，长内容允许纵向滚动 | Windows 1× / 1.25×；完整键盘、焦点和辅助技术巡检；长错误字符串 |
| 截图进入与标注 | 进入后尚未框选时已显示整屏灰色遮罩；放大镜、尺寸标签、边缘手柄、选区右下工具条均可见。修复首次复核发现的选区下缘图像溢出后复拍 | 物理双屏上按 F1、不同 DPI、跨屏停留与全屏应用 |
| 贴图 | 未选中时只有图片，没有工具栏；选中、取消标注、还原点击等由回归测试覆盖 | 此次没有逐个捕获右键菜单、选中与编辑状态 |
| 长截图 / GIF | 实际区域控制栏使用紧凑表面和明确的采集状态；未就绪操作呈禁用态 | Windows 原生控制栏与实际录制预览；本批目视使用合成内容 |
| OCR、系统菜单与提示 | 现有图中文字选择、退出、更新失败恢复等回归覆盖；系统界面继续由系统绘制 | 本批未逐个截图比对权限弹窗、每个右键项和 OCR 编辑状态，也未测英文全界面（当前应用为中文） |

按 5 分量表目视评估本批截屏：设置导航和排版 **4/5**（长页仍需滚动），主题辨识 **4/5**（未覆盖全部浮层与 Windows），截图和工具栏的布局 **4/5**（只在单屏原生环境观察）。分数是这组样本的设计检查，不是完整平台或无障碍认证。

实际窗口截图：[浅色设置](ui-screenshots/settings-light.png)、[深色最小窗口](ui-screenshots/settings-dark.png)、[未框选时的遮罩](ui-screenshots/capture-before-selection.png)、[选区右下标注条](ui-screenshots/capture-annotation.png)、[闲置贴图](ui-screenshots/pin.png)、[长截图](ui-screenshots/long-capture-controls.png)、[GIF 控制栏](ui-screenshots/gif-controls.png)。已等比缩小；图片内容均为内置示例。

可本地复现：

```sh
cargo build --locked
./target/debug/simple-snip --docs-demo settings-light
./target/debug/simple-snip --docs-demo settings-dark 7
./target/debug/simple-snip --docs-demo capture
./target/debug/simple-snip --docs-demo annotation
./target/debug/simple-snip --docs-demo pin
./target/debug/simple-snip --docs-demo long-controls
./target/debug/simple-snip --docs-demo gif
```

按顺序启动和关闭，每次只开一个预览。可将 `settings-dark` 的页码改为 `0`–`10`。预览使用独立程序锁和合成图，不占用 F1/F3，也不保存配置。真实双屏验收时，用正常应用按一次 F1：不移动鼠标时两屏均应变灰；换屏后直接框选，非当前屏仍应变灰；开始框选后跨屏不应创建第二个选区；Esc 应关闭两屏遮罩。重复验证正、负桌面原点和不同 DPI。
