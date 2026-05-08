<script setup lang="ts">
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/tauri'
import { computed, onMounted, onUnmounted, ref } from 'vue'

type BridgeSettings = {
  port: number
  accessToken: string
  allowedOrigins: string[]
  downloadDir: string
  keepDownloadedFiles: boolean
  confirmationRequired: boolean
}

type ServiceStatus = {
  running: boolean
  baseUrl: string
  lastError: string | null
}

type PrintJob = {
  id: string
  jobName: string
  status: string
  error: string | null
  requestedAt: string
}

type RuntimeSnapshot = {
  settings: BridgeSettings
  service: ServiceStatus
  jobs: PrintJob[]
  configDir: string
}

const state = ref<RuntimeSnapshot | null>(null)
const errorMessage = ref('')
const logFilePath = ref('')
let unlistenSnapshot: null | (() => void) = null

const lastJob = computed(() => state.value?.jobs?.[0] ?? null)
const statusText = computed(() => (state.value?.service.running ? '运行中' : '未启动'))

function applySnapshot(snapshot: RuntimeSnapshot) {
  state.value = snapshot
}

function formatJobStatus(status: string) {
  const labels: Record<string, string> = {
    downloading: '接收中',
    printing: '准备打印',
    dialogOpened: '已打开打印框',
    completed: '已完成',
    failed: '失败',
    cancelled: '已取消',
  }

  return labels[status] ?? status
}

async function refreshSnapshot() {
  errorMessage.value = ''
  try {
    applySnapshot(await invoke<RuntimeSnapshot>('get_runtime_snapshot'))
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function restartService() {
  errorMessage.value = ''
  try {
    applySnapshot(await invoke<RuntimeSnapshot>('restart_local_service'))
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function refreshLogPath() {
  try {
    logFilePath.value = await invoke<string>('get_log_file_path')
  } catch {
    logFilePath.value = ''
  }
}

async function openLogFile() {
  errorMessage.value = ''
  try {
    await invoke('open_log_file')
    await refreshLogPath()
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function hideWindow() {
  errorMessage.value = ''
  try {
    await invoke('hide_current_window')
  } catch (error) {
    errorMessage.value = String(error)
  }
}

onMounted(async () => {
  try {
    await refreshSnapshot()
    await refreshLogPath()
    unlistenSnapshot = await listen<RuntimeSnapshot>('print-bridge://snapshot', (event) => {
      applySnapshot(event.payload)
    })
  } catch (error) {
    errorMessage.value = String(error)
  }
})

onUnmounted(() => {
  unlistenSnapshot?.()
})
</script>

<template>
  <main class="plugin-shell">
    <section class="topbar">
      <div>
        <h1>辽易通打印插件 v0.1.2</h1>
        <p>{{ state?.service.baseUrl ?? '正在启动本地服务...' }}</p>
      </div>
      <span :class="['status-dot', state?.service.running ? 'is-ok' : 'is-bad']"></span>
    </section>

    <section class="status-panel">
      <div>
        <span>服务状态</span>
        <strong>{{ statusText }}</strong>
      </div>
      <div>
        <span>本地端口</span>
        <strong>{{ state?.settings.port ?? 12734 }}</strong>
      </div>
    </section>

    <section class="last-job">
      <span>最近任务</span>
      <strong>{{ lastJob ? lastJob.jobName : '暂无打印任务' }}</strong>
      <p v-if="lastJob">{{ formatJobStatus(lastJob.status) }}</p>
      <p v-if="lastJob?.error" class="error-text">{{ lastJob.error }}</p>
      <p v-else-if="state?.service.lastError" class="error-text">{{ state.service.lastError }}</p>
      <p v-else-if="errorMessage" class="error-text">{{ errorMessage }}</p>
    </section>

    <section class="actions">
      <button type="button" @click="refreshSnapshot">刷新状态</button>
      <button type="button" @click="restartService">重启服务</button>
      <button type="button" class="secondary" @click="openLogFile">打开日志</button>
      <button type="button" class="secondary" @click="hideWindow">隐藏</button>
    </section>

    <p class="footnote">{{ logFilePath || '日志路径加载中' }}</p>
  </main>
</template>
