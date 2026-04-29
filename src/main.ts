import { createApp } from 'vue'
import App from './App.vue'
import './style.css'

const routeParams = new URLSearchParams(window.location.hash.replace(/^#/, ''))

if (routeParams.get('printHost') === '1') {
  document.body.innerHTML = `
    <div class="print-shell">
      <header class="toolbar screen-only">
        <div>
          <strong id="job-title">正在准备打印文档...</strong>
          <p id="job-status">正在加载 PDF，请稍候。</p>
        </div>
        <div class="toolbar-actions">
          <button id="retry-print" type="button" disabled>打开系统打印</button>
          <button id="close-window" type="button" class="ghost">关闭窗口</button>
        </div>
      </header>

      <main id="pdf-pages" class="pdf-pages"></main>
    </div>
  `

  void import('./print-host')
} else {
  createApp(App).mount('#app')
}
