# 辽易通打印插件

辽易通打印插件是一个跨平台本地打印桥。用户安装插件后，业务网站可以通过本机 HTTP 服务提交 PDF，并调起系统打印选择框。

当前优先交付 Windows；国产化 Linux 会按统信 UOS、麒麟 Kylin，x86_64 和 ARM64 两类架构继续适配。macOS 保留后续适配入口。

## 运行方式

- 本地服务地址：`http://127.0.0.1:12734`
- 固定令牌：`liaoyitong-print-bridge-token-v1`
- CORS：允许所有网页来源
- 打印方式：网页提交 PDF 后直接调起系统打印选择框
- 程序形态：默认隐藏到系统托盘，状态窗口只保留运行状态和日志入口

## Web 接入

把 SDK 放到业务网站静态资源目录，然后引入：

```html
<script src="/liaoyitong-print-bridge.js"></script>
```

检测插件：

```html
<script>
  await LiaoyitongPrintBridge.ping()
</script>
```

打印 PDF：

```html
<script>
  await LiaoyitongPrintBridge.printPdf({
    jobName: '销售单打印',
    fileUrl: 'https://example.com/order.pdf'
  })
</script>
```

完整说明见 [docs/web-print-sdk.md](docs/web-print-sdk.md)。

## HTTP 接口

- `GET /printer/ping`
- `POST /printer/print`

请求头：

```text
X-Print-Bridge-Token: liaoyitong-print-bridge-token-v1
```

## 开发

```bash
npm install
npm run tauri:dev
```

## 构建

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

Windows 安装包：

```bash
bash ./build_win.sh
```

Linux deb 包后续按目标系统环境构建：

```bash
npm run build:linux
npm run build:linux:arm64
```
