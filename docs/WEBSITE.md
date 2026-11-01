# 项目官网

地址：https://junyi-deep.github.io/simple-snip/

官网使用独立的静态 HTML、CSS 和 JavaScript，以 Raycast 的深色视觉、大标题、产品展示与功能分区为参考；品牌、文案和截图来自 Simple Snip。无前端框架、CDN 字体或统计追踪。六种界面可以切换预览，功能配图可放大，原生 dialog 支持 Esc 关闭与焦点返回；窄屏布局和减少动态效果偏好均有适配。

## 本地预览

```powershell
python scripts/build-site.py
python -m http.server 4173 --bind 127.0.0.1 --directory dist/site
```

访问 http://127.0.0.1:4173/ 。源文件在 site/，与 README 共用 assets/screenshots/ 中的六张实际界面图片；输出在被 Git 忽略的 dist/site/。不要把私人的桌面截图加入公开素材。

## 部署

GitHub Pages 的构建方式为 GitHub Actions。main 分支修改 site/、共享素材、构建脚本或 pages.yml 时，Deploy project website 工作流构建静态目录、上传 Pages artifact 并部署；也可 workflow_dispatch 手动触发。发布官网无需运行 Rust 构建，原生应用仍由独立版本标签工作流发布。

所有站内资源使用相对路径，兼容 /simple-snip/ 项目路径。canonical、Open Graph、robots 和 sitemap 使用固定官网地址；迁移域名时应一起调整。

下载按钮默认指向 GitHub 最新 Release；若公开 Releases API 可用，则自动解析 Windows AMD64 ZIP 与 macOS ARM64 DMG 的真实下载地址。4 秒超时、网络错误或限流时保留 Release 页面链接。不会根据不可信响应跳转到第三方下载域名。

## 验证

本机 Chrome 无头浏览器验证六种预览、六个图片弹窗、Esc 关闭与原按钮焦点恢复、图片解码、320 / 390 / 600 / 900 / 1440px 无横向溢出、减少动态效果，以及下载 API 成功 / 离线回退；没有 JavaScript 页面异常。axe-core 的 WCAG 2 A / AA 与 2.1 AA 自动扫描发现并修复五处辅助文字对比度不足，最终桌面扫描无违规；自动扫描不能替代人工无障碍验收。检查时发现并修复了平板宽度下光晕溢出及首屏图片高度问题。实际应用截图已逐张目视检查，网页桌面和手机布局亦保存截图复查；这些检查不等于所有浏览器、读屏软件或全部设备的完整验收。


部署确认（2026-11-01）：[Pages 工作流 41316347209](https://github.com/junyi-deep/simple-snip/actions/runs/41316347209) 成功，来源提交 `14cfc2a`。正式地址的 index.html、CSS、JavaScript 与六张截图均 HTTP 200，SHA256 与本地构建产物逐一一致。在正式 /simple-snip/ 路径重新执行五种宽度、界面切换、图片弹窗和下载回退检查，全部通过。仓库 About 的 homepage 已指向该地址。


v0.0.13 发布后，使用正式站点和未拦截的真实 Releases API 再次确认：Windows 下载按钮解析到 v0.0.13 AMD64 ZIP，macOS 按钮解析到 v0.0.13 ARM64 DMG。
