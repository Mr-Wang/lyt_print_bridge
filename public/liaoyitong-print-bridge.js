(function (global) {
  'use strict'

  var DEFAULT_BASE_URL = 'http://127.0.0.1:12734'
  var TOKEN = 'liaoyitong-print-bridge-token-v1'

  function BridgeError(message, details) {
    this.name = 'LiaoyitongPrintBridgeError'
    this.message = message
    this.details = details || null
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, BridgeError)
    }
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

    var fileBase64 = hasFileBase64 ? stripBase64Prefix(options.fileBase64.trim()) : undefined

    return {
      jobName: options.jobName || 'PDF 打印任务',
      printerName: options.printerName || undefined,
      fileUrl: hasFileUrl ? options.fileUrl.trim() : undefined,
      fileBase64: fileBase64,
      fileName: options.fileName || undefined,
      contentType: options.contentType || 'pdf',
      copies: normalizeCopies(options.copies),
      pageRange: options.pageRange || undefined,
    }
  }

  function normalizeCopies(copies) {
    var value = Number(copies || 1)
    if (!Number.isFinite(value) || value < 1) {
      return 1
    }
    return Math.floor(value)
  }

  function stripBase64Prefix(value) {
    return String(value || '').replace(/^data:[^,]*;base64,/i, '')
  }

  async function request(path, options) {
    var baseUrl = normalizeBaseUrl(options && options.baseUrl)
    var requestOptions = Object.assign({}, options)
    delete requestOptions.baseUrl
    var response

    try {
      response = await fetch(baseUrl + path, requestOptions)
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

  async function listPrinters(options) {
    return request('/printer/list', {
      method: 'GET',
      headers: headers(),
      baseUrl: options && options.baseUrl,
    })
  }

  async function getJobs(options) {
    return request('/printer/jobs', {
      method: 'GET',
      headers: headers(),
      baseUrl: options && options.baseUrl,
    })
  }

  function fileToBase64(file) {
    if (!file) {
      return Promise.reject(new BridgeError('请选择 PDF 文件'))
    }

    return new Promise(function (resolve, reject) {
      var reader = new FileReader()
      reader.onload = function () {
        resolve(stripBase64Prefix(String(reader.result || '')))
      }
      reader.onerror = function () {
        reject(new BridgeError('读取本地 PDF 文件失败'))
      }
      reader.readAsDataURL(file)
    })
  }

  async function printLocalFile(file, options) {
    var fileBase64 = await fileToBase64(file)
    return printPdf(Object.assign({}, options, {
      jobName: options && options.jobName ? options.jobName : file.name,
      fileName: options && options.fileName ? options.fileName : file.name,
      fileBase64: fileBase64,
      fileUrl: undefined,
    }))
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
    listPrinters: listPrinters,
    getJobs: getJobs,
    printPdf: printPdf,
    printLocalFile: printLocalFile,
    fileToBase64: fileToBase64,
    BridgeError: BridgeError,
  }
})(window)
