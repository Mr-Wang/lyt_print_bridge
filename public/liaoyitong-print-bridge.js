(function (global) {
  'use strict'

  var DEFAULT_BASE_URL = 'http://127.0.0.1:12734'
  var TOKEN = 'liaoyitong-print-bridge-token-v1'

  function BridgeError(message, details) {
    this.name = 'LiaoyitongPrintBridgeError'
    this.message = message
    this.details = details || null
  }

  BridgeError.prototype = Object.create(Error.prototype)
  BridgeError.prototype.constructor = BridgeError

  function normalizeBaseUrl(baseUrl) {
    return String(baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, '')
  }

  function headers() {
    return {
      'Content-Type': 'application/json',
      'X-Print-Bridge-Token': TOKEN,
    }
  }

  async function parseResponse(response) {
    var text = await response.text()
    var data = null

    if (text) {
      try {
        data = JSON.parse(text)
      } catch (_error) {
        data = { raw: text }
      }
    }

    if (!response.ok) {
      var message = data && data.error ? data.error : '本地打印插件请求失败'
      throw new BridgeError(message, {
        status: response.status,
        response: data,
      })
    }

    return data
  }

  function normalizePrintOptions(options) {
    if (!options || typeof options !== 'object') {
      throw new BridgeError('printPdf 需要传入打印参数')
    }

    var hasFileUrl = typeof options.fileUrl === 'string' && options.fileUrl.trim()
    var hasFileBase64 = typeof options.fileBase64 === 'string' && options.fileBase64.trim()

    if ((hasFileUrl && hasFileBase64) || (!hasFileUrl && !hasFileBase64)) {
      throw new BridgeError('必须且只能提供 fileUrl 或 fileBase64')
    }

    return {
      jobName: options.jobName || 'PDF 打印任务',
      fileUrl: hasFileUrl ? options.fileUrl.trim() : undefined,
      fileBase64: hasFileBase64 ? options.fileBase64.trim().replace(/^data:application\/pdf;base64,/, '') : undefined,
      fileName: options.fileName || undefined,
      contentType: options.contentType || 'pdf',
      copies: options.copies || 1,
      pageRange: options.pageRange || undefined,
    }
  }

  async function request(path, options) {
    var baseUrl = normalizeBaseUrl(options && options.baseUrl)
    var response

    try {
      response = await fetch(baseUrl + path, options)
    } catch (error) {
      throw new BridgeError('未检测到本地打印插件，请确认辽易通打印插件已安装并正在运行。', {
        cause: String(error),
      })
    }

    return parseResponse(response)
  }

  async function ping(options) {
    return request('/printer/ping', {
      method: 'GET',
      headers: headers(),
      baseUrl: options && options.baseUrl,
    })
  }

  async function printPdf(options) {
    var payload = normalizePrintOptions(options)
    return request('/printer/print', {
      method: 'POST',
      headers: headers(),
      body: JSON.stringify(payload),
      baseUrl: options && options.baseUrl,
    })
  }

  global.LiaoyitongPrintBridge = {
    baseUrl: DEFAULT_BASE_URL,
    token: TOKEN,
    ping: ping,
    printPdf: printPdf,
    BridgeError: BridgeError,
  }
})(window)
