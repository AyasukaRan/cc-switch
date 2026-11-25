# 发布 Apple Silicon 魔改版

在 `feat/ran` 上递增稳定版本号，同时更新 `package.json`、Tauri 配置、Cargo 包版本及其锁文件。在 `src/whats-new/<版本>.json` 写入本次更新摘要，提交并推送分支。然后在该提交上创建并推送 `custom-v<版本>` 标签，如 `custom-v4.0.36`。

GitHub Actions 从标签提交构建 Apple Silicon 发行版，验证应用和更新包签名后发布 Release。ZIP 用于首次安装，应用内更新从本仓库 Latest Release 中的 `latest.json` 获取。macOS 应用使用 ad-hoc 签名，未经 Apple 公证。

仓库 Actions secret `CUSTOM_TAURI_SIGNING_PRIVATE_KEY` 保存 `pnpm tauri signer generate` 生成的私钥文件内容，配置中的公钥必须与之匹配。私钥若设置密码，另设 `CUSTOM_TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。后续版本沿用同一把密钥。

同一标签可在 Actions 界面重跑，流程会补全未发布的草稿。已发布的版本保留原文件。需要修正已发布的包时，递增版本后再打标签。
