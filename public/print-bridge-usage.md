# 辽易通打印桥第三方 JS 调用方法

## 引入

把 `liaoyitong-print-bridge.js` 放到业务页面可访问的位置，然后在页面中引入：

```html
<script src="./liaoyitong-print-bridge.js"></script>
```

默认本地服务地址是 `http://127.0.0.1:12734`，默认访问令牌已经内置在 JS 中。

## 检测插件是否运行

```html
<script>
async function checkPrintBridge() {
  try {
    const result = await LiaoyitongPrintBridge.ping()
    console.log('打印桥运行中:', result)
  } catch (error) {
    console.error('未检测到打印桥:', error.message)
  }
}
</script>
```

## 打印本地 PDF 文件

```html
<input id="pdfFile" type="file" accept="application/pdf" />
<button onclick="printLocalPdf()">打印本地 PDF</button>

<script>
async function printLocalPdf() {
  const file = document.getElementById('pdfFile').files[0]
  if (!file) {
    alert('请先选择 PDF 文件')
    return
  }

  try {
    const job = await LiaoyitongPrintBridge.printLocalFile(file)
    console.log('已提交打印任务:', job)
  } catch (error) {
    console.error('提交打印失败:', error.message, error.details)
  }
}
</script>
```

## 打印网络 PDF 地址

```html
<script>
async function printRemotePdf() {
  try {
    const job = await LiaoyitongPrintBridge.printPdf({
      jobName: '合同打印',
      fileName: 'contract.pdf',
      fileUrl: 'https://example.com/contract.pdf'
    })
    console.log('已提交打印任务:', job)
  } catch (error) {
    console.error('提交打印失败:', error.message, error.details)
  }
}
</script>
```

## 手动传 base64

```js
await LiaoyitongPrintBridge.printPdf({
  jobName: '本地上传 PDF',
  fileName: 'upload.pdf',
  fileBase64: 'JVBERi0xLjcK...'
})
```

`fileBase64` 可以是纯 base64，也可以是 `FileReader.readAsDataURL(file)` 得到的 `data:application/pdf;base64,...`。

## 可用方法

- `LiaoyitongPrintBridge.ping()`：检测本地服务。
- `LiaoyitongPrintBridge.printPdf(options)`：提交 PDF 打印任务，`fileUrl` 和 `fileBase64` 必须二选一。
- `LiaoyitongPrintBridge.printLocalFile(file, options)`：读取浏览器文件选择框中的 PDF 并提交打印。
- `LiaoyitongPrintBridge.fileToBase64(file)`：把浏览器选择的文件转成 base64。
- `LiaoyitongPrintBridge.listPrinters()`：读取本机打印机列表。
- `LiaoyitongPrintBridge.getJobs()`：读取最近打印任务。

## printPdf 参数

```js
{
  jobName: '显示在任务列表中的名称',
  fileName: '保存到本地的 PDF 文件名',
  fileUrl: 'https://example.com/a.pdf',
  fileBase64: 'PDF base64 内容',
  contentType: 'pdf',
  copies: 1,
  pageRange: '1-4',
  printerName: '可选打印机名称'
}
```

当前 Windows 版本会弹出系统打印机选择框，用户确认后交给 Windows 默认 PDF 打印链路处理。第三方页面只需要负责提交 PDF 文件或 PDF 地址。
