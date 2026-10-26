# 构建与本地打包

## macOS ARM64

安装 Rust Stable 1.99+ 与 Xcode；保持 Cargo.lock，禁止独立升级某一个 gpui-pre snapshot。

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --locked --release --target aarch64-apple-darwin
python3 scripts/notices.py --target aarch64-apple-darwin
bash packaging/macos/bundle.sh release
```

生成 `dist/Simple Snip.app`、`SimpleSnip_0.0.2_macos-arm64_unsigned.zip` 和同名 DMG。脚本断言 ARM64，嵌入原创图标和 Info.plist，校验 ad-hoc 签名；这是本地测试签名，**未公证**。原始 `assets/icons/app.svg` 与 `scripts/make-icons.py` 独立设计，无专有图片资源。

本地包重建会改变 ad-hoc 代码身份，已有 TCC 录屏条目可能仍绑定旧程序；请移除旧条目并重新添加当前 `.app`。本机 `security find-identity -v -p codesigning` 返回 0 个有效身份，不能交付持久的正式签名。脚本支持通过 `SIMPLE_SNIP_SIGN_IDENTITY` 指定已有有效身份（hardened runtime / timestamp），不会生成证书或降低系统权限要求。

正式分发需自行提供有效 Developer ID：对内部代码与最终 app 使用适当 hardened runtime 签名；通过 `xcrun notarytool submit ... --keychain-profile ... --wait` 提交，成功后 `xcrun stapler staple ...` 并验证 Gatekeeper。当前未提供证书或公证凭据，未执行这些正式分发步骤。不要去关闭系统安全策略。

## Windows AMD64

Windows 10/11、VS 2022 C++ Build Tools、Windows SDK、CMake、Rust MSVC。

```powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
python scripts/notices.py --target x86_64-pc-windows-msvc
./packaging/windows/package.ps1
```

生成标注 local-test 的便携 ZIP，GUI subsystem 不显示控制台，构建时嵌入图标；GPUI 自带 manifest 声明 PerMonitorV2。正式发布前检查实际 DLL / CRT 依赖和证书签名；安装包尚未制作。

本机从 macOS 执行完整 Windows GUI cargo check 时，GPUI build.rs 报 `NotAttempted("llvm-rc")`，因此未生成 Windows 可执行文件。平台库已能通过 `cargo check -p snip-platform --target x86_64-pc-windows-msvc`。应在 CI / Windows 原生环境完成资源编译、链接和运行测试，不能把 Rust target 已安装等同于平台构建完成。

## CI

`.github/workflows/build.yml` 只包含 Windows AMD64 / macOS ARM64：fmt、check、Clippy、单元测试、release、许可证通知与本地测试包。系统级测试需桌面 / 权限，标记 ignored 并提供人工步骤，不会在无头 CI 中冒充通过。

## GitHub 标签发布

公开仓库：junyi-deep/simple-snip。推送标签、PR 和手动分发会执行检查；普通 main 文档提交不重复构建。修改 Cargo 版本和 macOS Info.plist，执行 `python3 scripts/validate-release.py v0.0.1`（未来替换为新版本），提交并推送同名标签。Workflow 仅在两个原生平台的 fmt / check / Clippy / test / Release / 打包全部成功后创建 Release，生成每个安装包的 SHA256SUMS 并上传。GitHub Actions 只有发布 job 获得 contents:write，普通构建 contents:read。macOS runner 明确断言 ARM64。

首次包标注 unsigned；代码签名证书未提供，不能称公证发行版。机器原始验证日志保留本地且被 .gitignore 排除，仓库公开可复现步骤及结果摘要。

Windows 运行要求：Windows ZIP 中的程序实际导入 VCRUNTIME140.dll；首次运行前需安装微软官方 x64 Visual C++ v14 运行库（https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist ）。当前 ZIP 不捆绑该运行库。另导入系统 icuuc.dll，Windows 10 早于 1703 不满足这一依赖；该下界来自 DLL/API，不能替代完整最低系统真机验收。
