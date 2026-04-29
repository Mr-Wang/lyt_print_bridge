# 辽易通打印插件 Web 接入说明

辽易通打印插件安装后会在用户电脑本地启动一个 HTTP 服务：

```text
http://127.0.0.1:12734
```

网站只需要引入 `liaoyitong-print-bridge.js`，然后调用 `ping()` 和 `printPdf()`。

## 引入 SDK

```html
<script src="/liaoyitong-print-bridge.js"></script>
```

SDK 会自动使用固定令牌：

```text
liaoyitong-print-bridge-token-v1
```

## 检测插件是否可用

```html
<script>
  try {
    const status = await LiaoyitongPrintBridge.ping()
    console.log('打印插件在线', status)
  } catch (error) {
    console.log('打印插件未安装或未启动', error.message)
  }
</script>
```

## 打印远程 PDF

```html
<script>
  await LiaoyitongPrintBridge.printPdf({
    jobName: '销售单打印',
    fileUrl: 'https://example.com/order.pdf'
  })
</script>
```

## 打印 base64 PDF

```html
<script>
  await LiaoyitongPrintBridge.printPdf({
    jobName: '销售单打印',
    fileName: 'order.pdf',
    fileBase64: 'JVBERi0x...'
  })
</script>
```

## 参数

`printPdf(options)` 支持：

- `jobName?: string`
- `fileUrl?: string`
- `fileBase64?: string`
- `fileName?: string`
- `contentType?: string`
- `copies?: number`
- `pageRange?: string`

`fileUrl` 和 `fileBase64` 必须二选一，不能同时传。

## 错误处理

```html
<script>
  try {
    await LiaoyitongPrintBridge.printPdf({
      jobName: '销售单打印',
      fileUrl: 'https://example.com/order.pdf'
    })
  } catch (error) {
    alert(error.message)
  }
</script>
```

常见错误：

- `未检测到本地打印插件`：用户还没有安装插件，或插件没有运行。
- `必须且只能提供 fileUrl 或 fileBase64`：调用参数错误。
- `当前只支持 PDF 打印链路`：当前插件只处理 PDF。

## Demo

开发环境可打开：

```text
public/print-bridge-demo.html
```

正式部署时，把 `public/liaoyitong-print-bridge.js` 放到业务网站静态资源目录即可。
