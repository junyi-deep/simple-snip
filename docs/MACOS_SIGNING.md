# macOS 下载包的 Apple 验证

“Apple 无法验证 Simple Snip.app 是否包含恶意软件”是 Gatekeeper 的分发验证提示，与截图所需的「屏幕与系统音频录制」权限不同。当前公开的 `_unsigned` 包使用 ad-hoc 本地测试签名，没有 Developer ID 身份和公证票据；重装或重设录屏权限不会补上这些信息。

对外分发的修复路线是：**Developer ID Application 签名 → Apple 公证 Accepted → 附加公证票据 → Gatekeeper 校验 → 发布新安装包**。普通 Apple Development 签名只能用于开发，不能代替这一流程。不要关闭 Gatekeeper，也不要删除下载隔离属性来掩盖发行包的问题。

## 本地正式打包

需要 Apple Developer Program 账号、带私钥的有效 Developer ID Application 证书，以及具备公证权限的 Apple 账号。私钥和密码不要提交到 Git 或发送到聊天中。

1. 在钥匙串安装证书，确认 `security find-identity -v -p codesigning` 中存在 Developer ID Application 身份。
2. 交互式保存公证凭据（按 `notarytool` 提示输入 Apple ID、Team ID 和 App 专用密码）：

   ```sh
   xcrun notarytool store-credentials simple-snip-notary
   ```

3. 执行：

   ```sh
   SIMPLE_SNIP_SIGN_IDENTITY='Developer ID Application: 你的证书名称 (TEAMID)' \
   SIMPLE_SNIP_NOTARY_PROFILE=simple-snip-notary \
   SIMPLE_SNIP_DIST_DIR=dist/notarized \
   bash packaging/macos/bundle.sh release
   ```

使用独立输出目录，避免覆盖当前正在运行且已获录屏授权的本地包。脚本启用 hardened runtime 和安全时间戳，只接受 Developer ID Application。先公证并 staple `.app`，再重新生成 ZIP；随后签名、公证和 staple DMG。两个安装包都通过校验后才移出暂存目录，不会将上传成功当作公证成功。

成功输出 `SimpleSnip_<version>_macos-arm64.zip` / `.dmg`，没有 `_unsigned` 后缀。无签名身份时仍只能生成明确标记 `_unsigned` 的测试包；指定 Release 签名身份却缺少公证配置时直接失败。Debug 签名不要求公证。

## GitHub Actions

在仓库 Settings → Secrets and variables → Actions 添加以下 **Secrets**：

| 名称 | 内容 |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | 包含 Developer ID Application 证书及私钥的 P12，以 Base64 编码 |
| `MACOS_CERTIFICATE_PASSWORD` | P12 导出密码 |
| `MACOS_SIGN_IDENTITY` | 完整 Developer ID Application 身份名称 |
| `APPLE_ID` | 公证账号 Apple ID |
| `APPLE_TEAM_ID` | 证书对应 Team ID |
| `APPLE_APP_PASSWORD` | Apple App 专用密码，非账号登录密码 |

添加仓库 **Variable** `MACOS_SIGNED_RELEASE=true`，再推送新的版本标签。凭据只在 `v*` 标签的原生 macOS ARM64 作业使用，PR 和普通手动构建不导入凭据。临时 P12 会删除；签名钥匙串在 `always()` 步骤清理。启用后缺失凭据、签名失败、公证拒绝、超时或 Gatekeeper 校验失败都会阻止 Release，不降级为测试包。

未开启该变量时，流水线保持原来的双平台测试包构建，macOS 文件名仍带 `_unsigned`，下载后可能继续出现验证提示。只有配置凭据并实际发布通过公证的新包，才能完成对用户下载体验的修复。

## 分发验收

在没有旧安装记录的 Mac 上通过浏览器下载新包，断网测试启动，并实际验证截图、贴图和升级。保留以下本机检查结果：

```sh
codesign --verify --deep --strict 'dist/notarized/Simple Snip.app'
xcrun stapler validate 'dist/notarized/Simple Snip.app'
spctl --assess --type execute --verbose=2 'dist/notarized/Simple Snip.app'
```

本轮开发机查询为 **0 个有效签名身份**，因此未执行真实 Developer ID 签名、Apple 公证或公证包启动验收。脚本的隔离流程测试不代表 Apple 审核已通过。

依据：[Apple 公证说明](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)、[自定义公证流程](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)。ZIP 不能直接 staple，需重新压缩已经附加票据的应用。
