import { readBinaryFile } from '@tauri-apps/api/fs'
import { invoke } from '@tauri-apps/api/tauri'
import { GlobalWorkerOptions, getDocument } from 'pdfjs-dist'
import workerSrc from 'pdfjs-dist/build/pdf.worker.min?url'

GlobalWorkerOptions.workerSrc = workerSrc
document.documentElement.dataset.printHostScript = 'loaded'

type PrintJobDocument = {
  id: string
  jobName: string
  localFilePath: string
  printerName: string | null
  copies: number
  pageRange: string | null
}

const jobTitle = document.getElementById('job-title') as HTMLDivElement | null
const jobStatus = document.getElementById('job-status') as HTMLParagraphElement | null
const retryButton = document.getElementById('retry-print') as HTMLButtonElement | null
const closeButton = document.getElementById('close-window') as HTMLButtonElement | null
const pdfPages = document.getElementById('pdf-pages') as HTMLDivElement | null

let currentJobId = ''
let triggerInFlight = false

function toLogText(message: string, detail?: unknown) {
  if (detail === undefined) {
    return message
  }

  try {
    return `${message} ${JSON.stringify(detail, (_key, value) => {
      if (value instanceof Error) {
        return {
          name: value.name,
          message: value.message,
          stack: value.stack,
        }
      }
      return value
    })}`
  } catch {
    return `${message} ${String(detail)}`
  }
}

async function writeLog(message: string, detail?: unknown) {
  const text = toLogText(message, detail)
  console.info(`[print-host] ${text}`)

  try {
    await invoke('frontend_log', {
      source: 'print-host',
      message: text,
    })
  } catch (error) {
    console.warn('[print-host] failed to write backend log', error)
  }
}

function setStatus(message: string) {
  if (jobStatus) {
    jobStatus.textContent = message
  }
}

function getJobIdFromHash() {
  const hash = new URLSearchParams(window.location.hash.replace(/^#/, ''))
  return hash.get('job')?.trim() || ''
}

async function renderJob(jobId: string) {
  await writeLog('renderJob started', { jobId, href: window.location.href })
  currentJobId = jobId
  if (retryButton) {
    retryButton.disabled = true
  }
  if (pdfPages) {
    pdfPages.innerHTML = ''
  }
  setStatus('正在读取打印任务...')

  try {
    await writeLog('invoke get_print_job_document')
    const job = await invoke<PrintJobDocument>('get_print_job_document', { jobId })
    await writeLog('print job document received', {
      id: job.id,
      jobName: job.jobName,
      localFilePath: job.localFilePath,
      printerName: job.printerName,
      copies: job.copies,
      pageRange: job.pageRange,
    })
    document.title = `打印任务 - ${job.jobName}`
    if (jobTitle) {
      jobTitle.textContent = job.jobName
    }

    await renderPdfDocument(job.localFilePath)
    await waitForPrintLayout()
    if (retryButton) {
      retryButton.disabled = false
    }
    setStatus('文档已加载，正在打开系统打印对话框...')
    await writeLog('PDF rendered, triggering print dialog')
    await triggerPrintDialog(jobId)
  } catch (error) {
    const message = String(error)
    await writeLog('renderJob failed', { jobId, error: message })
    setStatus(message)
    if (retryButton) {
      retryButton.disabled = false
    }
    await reportPrintError(jobId, message)
  }
}

async function renderPdfDocument(localFilePath: string) {
  if (!pdfPages) {
    throw new Error('打印窗口缺少 PDF 容器元素。')
  }

  await writeLog('reading local PDF file', { localFilePath })
  const pdfBytes = await readBinaryFile(localFilePath)
  await writeLog('local PDF file read', { byteLength: pdfBytes.byteLength })

  await writeLog('loading PDF document', { workerSrc })
  const loadingTask = getDocument({ data: pdfBytes })
  const pdf = await loadingTask.promise
  await writeLog('PDF document loaded', { numPages: pdf.numPages })

  if (!pdf.numPages) {
    throw new Error('PDF 文件没有可打印页面。')
  }

  for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber += 1) {
    await writeLog('rendering PDF page', { pageNumber })
    const page = await pdf.getPage(pageNumber)
    const viewport = page.getViewport({ scale: 1.35 })
    const canvas = document.createElement('canvas')
    const context = canvas.getContext('2d')
    if (!context) {
      throw new Error('创建 PDF 画布失败。')
    }

    canvas.width = viewport.width
    canvas.height = viewport.height
    canvas.className = 'pdf-canvas'

    await page.render({
      canvasContext: context,
      viewport,
    }).promise

    const pageWrapper = document.createElement('section')
    pageWrapper.className = 'page-wrapper'
    pageWrapper.appendChild(canvas)
    pdfPages.appendChild(pageWrapper)
    await writeLog('PDF page rendered', {
      pageNumber,
      width: canvas.width,
      height: canvas.height,
    })
  }
}

function nextAnimationFrame() {
  return new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => resolve())
  })
}

async function waitForPrintLayout() {
  const fontFaceSet = (document as Document & {
    fonts?: { ready?: Promise<unknown> }
  }).fonts

  if (fontFaceSet?.ready) {
    await fontFaceSet.ready
  }

  await nextAnimationFrame()
  await nextAnimationFrame()
  await new Promise<void>((resolve) => {
    window.setTimeout(resolve, 180)
  })

  await writeLog('print layout settled', {
    pageCount: pdfPages?.querySelectorAll('.page-wrapper').length ?? 0,
    bodyWidth: document.body.scrollWidth,
    bodyHeight: document.body.scrollHeight,
  })
}

async function triggerPrintDialog(jobId: string) {
  if (triggerInFlight) {
    await writeLog('triggerPrintDialog skipped because already in flight', { jobId })
    return
  }

  triggerInFlight = true
  if (retryButton) {
    retryButton.disabled = true
  }

  try {
    await writeLog('invoke notify_print_ready', { jobId })
    await invoke('notify_print_ready', { jobId })
    await writeLog('notify_print_ready finished', { jobId })
    setStatus('系统打印对话框已触发。')
    await writeLog('print host kept alive for system print dialog', { jobId })
  } catch (error) {
    const message = String(error)
    await writeLog('triggerPrintDialog failed', { jobId, error: message })
    setStatus(message)
    await reportPrintError(jobId, message)
  } finally {
    triggerInFlight = false
    if (retryButton) {
      retryButton.disabled = false
    }
  }
}

async function reportPrintError(jobId: string, message: string) {
  if (!jobId) {
    return
  }

  try {
    await invoke('report_print_error', { jobId, message })
    await writeLog('report_print_error finished', { jobId, message })
  } catch {
    // keep the window usable even if status sync fails
  }
}

retryButton?.addEventListener('click', async () => {
  if (!currentJobId) {
    return
  }
  await writeLog('retry button clicked', { currentJobId })
  setStatus('正在重新打开系统打印对话框...')
  await triggerPrintDialog(currentJobId)
})

closeButton?.addEventListener('click', async () => {
  await writeLog('close button clicked')
  await invoke('close_current_window')
})

window.addEventListener('hashchange', async () => {
  const nextJobId = getJobIdFromHash()
  await writeLog('hashchange received', { hash: window.location.hash, nextJobId })
  if (!nextJobId) {
    setStatus('未找到打印任务编号。')
    return
  }
  await renderJob(nextJobId)
})

window.addEventListener('error', (event) => {
  void writeLog('window error', {
    message: event.message,
    filename: event.filename,
    lineno: event.lineno,
    colno: event.colno,
    error: event.error,
  })
})

window.addEventListener('unhandledrejection', (event) => {
  void writeLog('unhandled promise rejection', {
    reason: event.reason,
  })
})

async function startPrintHost() {
  await writeLog('DOMContentLoaded', {
    href: window.location.href,
    hash: window.location.hash,
    userAgent: navigator.userAgent,
    elements: {
      jobTitle: Boolean(jobTitle),
      jobStatus: Boolean(jobStatus),
      retryButton: Boolean(retryButton),
      closeButton: Boolean(closeButton),
      pdfPages: Boolean(pdfPages),
    },
  })

  const style = document.createElement('style')
  style.textContent = `
    :root {
      color-scheme: light;
      font-family: "PingFang SC", "Microsoft YaHei", Inter, sans-serif;
      background: #eef2f7;
      color: #111827;
    }
    * { box-sizing: border-box; }
    body { margin: 0; background: #eef2f7; }
    button {
      border: 0;
      border-radius: 10px;
      background: #2563eb;
      color: #fff;
      padding: 10px 14px;
      cursor: pointer;
      font: inherit;
    }
    button:disabled { opacity: 0.6; cursor: wait; }
    button.ghost { background: #e5e7eb; color: #111827; }
    .print-shell { min-height: 100vh; }
    .toolbar {
      position: sticky;
      top: 0;
      z-index: 10;
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 16px;
      padding: 14px 18px;
      background: rgba(255,255,255,0.96);
      border-bottom: 1px solid #dbe2ea;
    }
    .toolbar p { margin: 6px 0 0; color: #4b5563; }
    .toolbar-actions { display: flex; gap: 10px; }
    .pdf-pages {
      max-width: 960px;
      min-height: 180px;
      margin: 0 auto;
      padding: 20px 16px 32px;
    }
    .page-wrapper {
      margin: 0 auto 18px;
      width: fit-content;
      box-shadow: 0 12px 40px rgba(15, 23, 42, 0.12);
      background: #fff;
    }
    .pdf-canvas {
      display: block;
      max-width: min(100%, 900px);
      height: auto;
    }
    @media print {
      body { background: #fff; }
      .screen-only { display: none !important; }
      .pdf-pages { max-width: none; margin: 0; padding: 0; }
      .page-wrapper {
        margin: 0;
        box-shadow: none;
        break-after: page;
        page-break-after: always;
      }
      .page-wrapper:last-child {
        break-after: auto;
        page-break-after: auto;
      }
      .pdf-canvas {
        max-width: 100%;
        width: 100%;
      }
    }
  `
  document.head.appendChild(style)
  await writeLog('print-host style injected')

  const initialJobId = getJobIdFromHash()
  await writeLog('initial job id parsed', { initialJobId })
  if (!initialJobId) {
    setStatus('未找到打印任务编号。')
    return
  }

  await renderJob(initialJobId)
}

if (document.readyState === 'loading') {
  window.addEventListener('DOMContentLoaded', () => {
    void startPrintHost()
  }, { once: true })
} else {
  void startPrintHost()
}
