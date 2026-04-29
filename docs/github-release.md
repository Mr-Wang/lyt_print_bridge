# GitHub 打版说明

这个项目可以用 GitHub Actions 做两类打版：

- `Release`：GitHub 官方 runner 自动打 Windows x64 安装包和 Ubuntu 22.04 x64 deb。适合常规发布和 CI 留档。
- `Domestic self-hosted build`：跑在你自己的统信 UOS、银河麒麟等国产化机器上，适合真实国产化环境验收。

## 首次上传到 GitHub

本机初始化并推送：

```bash
git init
git add .
git commit -m "Initial release workflow"
git branch -M main
git remote add origin git@github.com:<your-org-or-user>/lyt_print_bridge.git
git push -u origin main
```

如果你不用 SSH，也可以把 `origin` 换成 GitHub 页面给出的 HTTPS 地址。

## 触发正式发布

更新版本号后打 tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

GitHub 会自动运行 `.github/workflows/release.yml`，并把 Tauri 产物挂到对应 Release。

也可以在 GitHub 仓库页面进入 `Actions`，选择 `Release`，点 `Run workflow` 手动打包。

## 国产化 runner 准备

GitHub 官方 Linux runner 不是统信/麒麟环境。要做真正国产化打版，需要准备目标系统机器，然后在仓库：

1. 进入 `Settings` -> `Actions` -> `Runners`。
2. 新增 self-hosted runner，按 GitHub 页面命令下载安装到目标机器。
3. 给 x64 机器加标签：`linux`、`X64`、`domestic`。
4. 给 ARM64 机器加标签：`linux`、`ARM64`、`domestic`。
5. 在目标机预装 Node.js 20、Rust stable、Tauri Linux 依赖、系统打包工具。

目标机常用依赖示例：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  curl \
  file \
  libgtk-3-dev \
  libwebkit2gtk-4.0-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

配置完 runner 后，在 `Actions` 里手动运行 `Domestic self-hosted build`，产物会上传到 workflow artifacts。

## 需要你提供或确认的东西

- GitHub 仓库归属：个人账号还是组织。
- 仓库名：建议 `lyt_print_bridge` 或 `liaoyitong-print-bridge`。
- 是否私有仓库。
- 如果要真实国产化打版，需要一台统信/麒麟 x64 或 ARM64 机器接入 self-hosted runner。
- 如果后续要代码签名，需要提供签名证书和 GitHub Secrets。
