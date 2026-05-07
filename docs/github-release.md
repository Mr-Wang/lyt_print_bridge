# GitHub 免费云打版说明

这个项目的 GitHub Actions 只打 Linux deb，Windows 安装包继续本地构建。

## 免费方案边界

- 仓库需要设置为 public，才能使用 GitHub 免费云 runner 打 x64 和 ARM64。
- GitHub 免费云 runner 是 Ubuntu，不是统信 UOS 或银河麒麟。
- 产物定位为 Ubuntu 通用 deb，需要再拿到 UOS/Kylin x86_64、ARM64 实机验证。
- 如果以后必须在 UOS/Kylin 原生环境构建，需要改用 self-hosted runner 或国产化云主机。

## 首次上传到 GitHub

```bash
git remote add origin https://github.com/Mr-Wang/lyt_print_bridge.git
git push -u origin main
```

如果远端已经存在 `origin`，改用：

```bash
git remote set-url origin https://github.com/Mr-Wang/lyt_print_bridge.git
git push -u origin main
```

## 触发 Linux deb 发布

推送 `v*` tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

GitHub 会运行 `.github/workflows/release.yml`，并在同一个 GitHub Release 中生成：

- `liaoyitong-print-bridge_0.1.0_linux_x64.deb`
- `liaoyitong-print-bridge_0.1.0_linux_arm64.deb`

Release 默认是 draft。确认产物可用后，在 GitHub 页面手动发布即可。

## 本地 Windows 打包

Windows 不走 GitHub Actions：

```bash
bash ./build_win.sh
```

## 国产化实机验证

下载 Release 里的 deb 后，在目标机器安装并验证：

```bash
sudo apt install ./liaoyitong-print-bridge_0.1.0_linux_x64.deb
```

验证重点：

- `LiaoyitongPrintBridge.ping()` 成功。
- `fileUrl` 和 `fileBase64` 都能提交打印任务。
- 系统打印选择框能弹出。
- 关闭状态窗口后插件仍常驻托盘。
- 桌面重新登录后插件能自动启动。
