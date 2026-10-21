<template>
  <div class="settings-container">
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark">J</div>
        <div class="brand-text">
          <span class="brand-name">Jpixel</span>
          <span class="brand-sub">截图工具</span>
        </div>
      </div>

      <nav class="nav" aria-label="设置导航">
        <button
          type="button"
          class="nav-item"
          :class="{ active: activeSection === 'general' }"
          @click="activeSection = 'general'"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v11a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 17.5v-11Z" />
            <path d="M8 9h8M8 12h8M8 15h5" />
          </svg>
          <span>常规</span>
        </button>
        <button
          type="button"
          class="nav-item"
          :class="{ active: activeSection === 'shortcuts' }"
          @click="activeSection = 'shortcuts'"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="3" y="6" width="18" height="12" rx="2.5" />
            <path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10" />
          </svg>
          <span>快捷键</span>
        </button>
      </nav>

      <div class="sidebar-footer">v{{ version }}</div>
    </aside>

    <main class="content">
      <header class="page-header">
        <h1>{{ activeSection === "general" ? "常规" : "快捷键" }}</h1>
        <p>
          {{
            activeSection === "general"
              ? "管理 Jpixel 的启动与基础行为。"
              : "点击键帽后直接按下新的组合键，立即生效。"
          }}
        </p>
      </header>

      <section v-if="activeSection === 'general'" class="settings-card">
        <div class="card-header">
          <h2>启动与行为</h2>
          <p>登录后自动运行，并决定截图完成后的默认动作。</p>
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">开机自启动</div>
            <p class="setting-desc">登录 Windows 后自动运行 Jpixel，截图热键随时可用。</p>
          </div>
          <label class="switch">
            <input
              type="checkbox"
              v-model="autoLaunch"
              aria-label="开机自启动"
              @change="onAutoLaunchChange"
            />
            <span class="slider"></span>
          </label>
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">双击防误触</div>
            <p class="setting-desc">需要快速连按两次截图快捷键才会触发。</p>
          </div>
          <label class="switch">
            <input
              type="checkbox"
              v-model="doublePress"
              aria-label="双击防误触"
              @change="onToggle"
            />
            <span class="slider"></span>
          </label>
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">双击速度</div>
            <p class="setting-desc">两次按键之间的最大间隔；越慢越容易触发。</p>
          </div>
          <SettingsSelect
            v-model="doublePressSpeed"
            :options="doublePressSpeedOptions"
            aria-label="双击速度"
            @change="onDoublePressSpeedChange"
          />
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">贴图数量上限</div>
            <p class="setting-desc">同时保留的贴图数量；超出时会提示你确认替换最早的一张。</p>
          </div>
          <SettingsSelect
            v-model="pinLimit"
            :options="pinLimitOptions"
            aria-label="贴图数量上限"
            @change="onPinLimitChange"
          />
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">默认操作</div>
            <p class="setting-desc">双击或按 Enter 时执行的操作。</p>
          </div>
          <SettingsSelect
            v-model="defaultAction"
            :options="defaultActionOptions"
            aria-label="默认操作"
            @change="onDefaultActionChange"
          />
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">十字辅助线</div>
            <p class="setting-desc">在光标处显示横向/纵向辅助线，方便对齐边缘。</p>
          </div>
          <label class="switch">
            <input
              type="checkbox"
              v-model="crosshairGuides"
              aria-label="十字辅助线"
              @change="onCrosshairChange"
            />
            <span class="slider"></span>
          </label>
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">选后操作</div>
            <p class="setting-desc">拖动重选保持十字准星；调整大小显示控制点。</p>
          </div>
          <SettingsSelect
            v-model="selectionMode"
            :options="selectionModeOptions"
            aria-label="选后操作"
            @change="onSelectionModeChange"
          />
        </div>
      </section>

      <template v-if="activeSection === 'shortcuts'">
        <div v-if="hotkeyError" class="error-banner" role="alert">
          {{ hotkeyError }}
        </div>

        <section class="settings-card">
          <div class="card-header">
            <h2>截图</h2>
            <p>设置唤起遮罩、复制、保存和贴图的快捷键。</p>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <div class="setting-label">区域截图</div>
              <p class="setting-desc">按下后冻结当前屏幕并进入框选。</p>
            </div>
            <div
              class="hotkey-input"
              :class="{ recording: recordingMode === 'screenshot' }"
              role="button"
              tabindex="0"
              :aria-label="`设置区域截图快捷键，当前 ${screenshotHotkey}`"
              @click="startRecording('screenshot')"
              @keydown="(e) => onKeyDown(e, 'screenshot')"
              @blur="stopRecording"
            >
              <span v-if="recordingMode === 'screenshot'" class="recording-label">
                按下快捷键…
              </span>
              <template v-else>
                <template
                  v-for="(key, index) in hotkeyKeys(screenshotHotkey)"
                  :key="`screenshot-${key}-${index}`"
                >
                  <kbd>{{ key }}</kbd>
                  <span
                    v-if="index < hotkeyKeys(screenshotHotkey).length - 1"
                    class="key-plus"
                  >+</span>
                </template>
              </template>
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <div class="setting-label">复制</div>
              <p class="setting-desc">框选后按下快捷键，直接复制到剪贴板。</p>
            </div>
            <div
              class="hotkey-input"
              :class="{ recording: recordingMode === 'copy' }"
              role="button"
              tabindex="0"
              :aria-label="`设置复制快捷键，当前 ${copyHotkey}`"
              @click="startRecording('copy')"
              @keydown="(e) => onKeyDown(e, 'copy')"
              @blur="stopRecording"
            >
              <span v-if="recordingMode === 'copy'" class="recording-label">
                按下快捷键…
              </span>
              <template v-else>
                <template
                  v-for="(key, index) in hotkeyKeys(copyHotkey)"
                  :key="`copy-${key}-${index}`"
                >
                  <kbd>{{ key }}</kbd>
                  <span v-if="index < hotkeyKeys(copyHotkey).length - 1" class="key-plus">+</span>
                </template>
              </template>
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <div class="setting-label">保存</div>
              <p class="setting-desc">框选后按下快捷键，弹出保存对话框。</p>
            </div>
            <div
              class="hotkey-input"
              :class="{ recording: recordingMode === 'save' }"
              role="button"
              tabindex="0"
              :aria-label="`设置保存快捷键，当前 ${saveHotkey}`"
              @click="startRecording('save')"
              @keydown="(e) => onKeyDown(e, 'save')"
              @blur="stopRecording"
            >
              <span v-if="recordingMode === 'save'" class="recording-label">
                按下快捷键…
              </span>
              <template v-else>
                <template
                  v-for="(key, index) in hotkeyKeys(saveHotkey)"
                  :key="`save-${key}-${index}`"
                >
                  <kbd>{{ key }}</kbd>
                  <span v-if="index < hotkeyKeys(saveHotkey).length - 1" class="key-plus">+</span>
                </template>
              </template>
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <div class="setting-label">贴图</div>
              <p class="setting-desc">框选后按下快捷键，把选区贴到屏幕上。</p>
            </div>
            <div
              class="hotkey-input"
              :class="{ recording: recordingMode === 'pin' }"
              role="button"
              tabindex="0"
              :aria-label="`设置贴图快捷键，当前 ${pinHotkey}`"
              @click="startRecording('pin')"
              @keydown="(e) => onKeyDown(e, 'pin')"
              @blur="stopRecording"
            >
              <span v-if="recordingMode === 'pin'" class="recording-label">
                按下快捷键…
              </span>
              <template v-else>
                <template
                  v-for="(key, index) in hotkeyKeys(pinHotkey)"
                  :key="`pin-${key}-${index}`"
                >
                  <kbd>{{ key }}</kbd>
                  <span v-if="index < hotkeyKeys(pinHotkey).length - 1" class="key-plus">+</span>
                </template>
              </template>
            </div>
          </div>
        </section>

      </template>

      <footer class="page-footer">
        <span>BUG 反馈：2025886838@qq.com</span>
        <span>由见桑 &amp; DeepSeek 制作</span>
      </footer>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import {
  getAutoLaunch,
  setAutoLaunch,
  getDoublePressEnabled,
  setDoublePressEnabled,
  getScreenshotHotkey,
  setScreenshotHotkey,
  getCopyHotkey,
  setCopyHotkey,
  getSaveHotkey,
  setSaveHotkey,
  getPinHotkey,
  setPinHotkey,
  getPinLimit,
  setPinLimit,
  getDefaultAction,
  setDefaultAction,
  getCrosshairGuides,
  setCrosshairGuides,
  getSelectionMode,
  setSelectionMode,
  getDoublePressSpeed,
  setDoublePressSpeed,
  type SelectionMode,
  type DoublePressSpeed,
  type PinLimit,
} from "../api/ipc";
import { formatHotkey } from "../utils/hotkey";
import SettingsSelect from "../components/SettingsSelect.vue";

const version = "1.2.0";
const activeSection = ref("general");
const autoLaunch = ref(false);
const doublePress = ref(false);
const screenshotHotkey = ref("Alt+X");
const copyHotkey = ref("Ctrl+C");
const saveHotkey = ref("Ctrl+S");
const pinHotkey = ref("Ctrl+T");
const defaultAction = ref("save_and_edit");
const crosshairGuides = ref(true);
const selectionMode = ref("reselect");
const doublePressSpeed = ref("normal");
const pinLimit = ref("eight");
const recordingMode = ref<string | null>(null);
const hotkeyError = ref<string | null>(null);

const actionLabels: Record<string, string> = {
  save_and_edit: "保存并编辑",
  copy: "复制到剪贴板",
  save: "保存文件",
  pin: "贴图",
};

const defaultActionOptions = Object.entries(actionLabels).map(([value, label]) => ({
  value,
  label,
}));

const selectionModeOptions = [
  { value: "reselect", label: "拖动重选" },
  { value: "resize", label: "调整大小" },
];

const doublePressSpeedOptions = [
  { value: "slow", label: "慢（700ms）" },
  { value: "normal", label: "标准（500ms）" },
  { value: "fast", label: "快（300ms）" },
];

const pinLimitOptions = [
  { value: "four", label: "2 张" },
  { value: "eight", label: "4 张" },
  { value: "sixteen", label: "6 张" },
  { value: "thirty_two", label: "8 张" },
];

function hotkeyKeys(value: string): string[] {
  return value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
}

onMounted(async () => {
  try {
    autoLaunch.value = await getAutoLaunch();
    doublePress.value = await getDoublePressEnabled();
    screenshotHotkey.value = await getScreenshotHotkey();
    copyHotkey.value = await getCopyHotkey();
    saveHotkey.value = await getSaveHotkey();
    pinHotkey.value = await getPinHotkey();
    defaultAction.value = await getDefaultAction();
    crosshairGuides.value = await getCrosshairGuides();
    selectionMode.value = await getSelectionMode();
    doublePressSpeed.value = await getDoublePressSpeed();
    pinLimit.value = await getPinLimit();
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
});

function onAutoLaunchChange() {
  setAutoLaunch(autoLaunch.value).catch((err: unknown) => {
    console.error("Failed to set auto launch:", err);
  });
}

function onToggle() {
  setDoublePressEnabled(doublePress.value).catch((err: unknown) => {
    console.error("Failed to set double press:", err);
  });
}

function onDefaultActionChange() {
  setDefaultAction(defaultAction.value).catch((err: unknown) => {
    console.error("Failed to set default action:", err);
  });
}

function onCrosshairChange() {
  setCrosshairGuides(crosshairGuides.value).catch((err: unknown) => {
    console.error("Failed to set crosshair guides:", err);
  });
}

function onSelectionModeChange() {
  setSelectionMode(selectionMode.value as SelectionMode).catch((err: unknown) => {
    console.error("Failed to set selection mode:", err);
  });
}

function onDoublePressSpeedChange() {
  setDoublePressSpeed(doublePressSpeed.value as DoublePressSpeed).catch((err: unknown) => {
    console.error("Failed to set double press speed:", err);
  });
}

function onPinLimitChange() {
  setPinLimit(pinLimit.value as PinLimit).catch((err: unknown) => {
    console.error("Failed to set pin limit:", err);
  });
}

function startRecording(mode: string) {
  hotkeyError.value = null;
  recordingMode.value = mode;
}

function stopRecording() {
  recordingMode.value = null;
}

interface SetterEntry {
  ref: typeof screenshotHotkey;
  setter: (hotkey: string) => Promise<void>;
}

const setters: Record<string, SetterEntry> = {
  screenshot: { ref: screenshotHotkey, setter: setScreenshotHotkey },
  copy: { ref: copyHotkey, setter: setCopyHotkey },
  save: { ref: saveHotkey, setter: setSaveHotkey },
  pin: { ref: pinHotkey, setter: setPinHotkey },
};

function onKeyDown(e: KeyboardEvent, mode: string) {
  if (recordingMode.value !== mode) return;
  e.preventDefault();
  e.stopPropagation();

  const hotkeyStr = formatHotkey(e);
  if (!hotkeyStr) return;

  recordingMode.value = null;

  const entry = setters[mode];
  if (!entry) return;

  const previous = entry.ref.value;
  entry
    .setter(hotkeyStr)
    .then(() => {
      entry.ref.value = hotkeyStr;
      hotkeyError.value = null;
    })
    .catch((err: unknown) => {
      entry.ref.value = previous;
      hotkeyError.value = `快捷键设置失败：${String(err)}`;
      console.error(`Failed to set ${mode} hotkey:`, err);
    });
}
</script>

<style scoped>
:global(html),
:global(body),
:global(#app) {
  width: 100%;
  height: 100%;
  margin: 0;
}

.settings-container {
  --bg: #f3f3f3;
  --sidebar: #f9f9f9;
  --card: #ffffff;
  --text: #1b1b1b;
  --muted: #6b6b6b;
  --border: #e5e5e5;
  --accent: #2f6fed;
  --accent-soft: rgba(47, 111, 237, 0.1);
  --danger-bg: #fdecea;
  --danger: #a12622;
  --shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
  --row-hover: rgba(47, 111, 237, 0.035);

  display: flex;
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  color-scheme: light dark;
  font-family: "Segoe UI Variable", "Segoe UI", system-ui, -apple-system, sans-serif;
  -webkit-font-smoothing: antialiased;
}

.settings-container,
.settings-container * {
  box-sizing: border-box;
}

@media (prefers-color-scheme: dark) {
  .settings-container {
    --bg: #1f1f1f;
    --sidebar: #252525;
    --card: #2b2b2b;
    --text: #f2f2f2;
    --muted: #a6a6a6;
    --border: #3a3a3a;
    --accent: #4cc2ff;
    --accent-soft: rgba(76, 194, 255, 0.12);
    --danger-bg: #3b2323;
    --danger: #ffb4ab;
    --shadow: 0 1px 3px rgba(0, 0, 0, 0.22);
    --row-hover: rgba(76, 194, 255, 0.055);
  }
}

.sidebar {
  display: flex;
  flex: 0 0 220px;
  flex-direction: column;
  min-width: 0;
  height: 100%;
  background: var(--sidebar);
  border-right: 1px solid var(--border);
}

.brand {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 26px 20px 20px;
}

.brand-mark {
  display: grid;
  flex: 0 0 34px;
  width: 34px;
  height: 34px;
  place-items: center;
  border-radius: 10px;
  background: var(--accent);
  color: #ffffff;
  font-size: 17px;
  font-weight: 700;
}

.brand-text {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.brand-name {
  font-size: 15px;
  font-weight: 650;
  letter-spacing: -0.01em;
}

.brand-sub {
  color: var(--muted);
  font-size: 11px;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 8px 10px;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 13.5px;
  text-align: left;
  cursor: pointer;
  transition: background 0.16s ease, color 0.16s ease, transform 0.12s ease;
}

.nav-item:active {
  transform: scale(0.985);
}

.nav-item svg {
  width: 18px;
  height: 18px;
  flex: 0 0 18px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.6;
}

.nav-item:hover {
  background: rgba(128, 128, 128, 0.1);
  color: var(--text);
}

.nav-item.active {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}

.sidebar-footer {
  margin-top: auto;
  padding: 14px 20px;
  border-top: 1px solid var(--border);
  color: var(--muted);
  font-size: 11px;
}

.content {
  flex: 1;
  min-width: 0;
  padding: 36px 40px 28px;
  overflow-y: auto;
  scrollbar-color: var(--border) transparent;
}

.page-header {
  max-width: 720px;
  margin-bottom: 22px;
  animation: fade-up 0.26s ease both;
}

.page-header h1 {
  margin: 0 0 6px;
  font-size: 27px;
  font-weight: 650;
  letter-spacing: -0.025em;
}

.page-header p {
  margin: 0;
  color: var(--muted);
  font-size: 13.5px;
  line-height: 1.55;
}

.settings-card {
  max-width: 720px;
  margin-bottom: 18px;
  overflow: visible;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--card);
  box-shadow: var(--shadow);
  animation: card-in 0.28s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.settings-card:hover {
  border-color: color-mix(in srgb, var(--accent) 18%, var(--border));
  box-shadow: 0 3px 10px rgba(0, 0, 0, 0.055);
}

.card-header {
  padding: 18px 20px 12px;
}

.card-header h2 {
  margin: 0 0 4px;
  font-size: 14px;
  font-weight: 650;
}

.card-header p {
  margin: 0;
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.5;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 28px;
  padding: 18px 20px;
  transition: background 0.15s ease;
}

.setting-row:hover {
  background: var(--row-hover);
}

.setting-row + .setting-row {
  border-top: 1px solid var(--border);
}

.setting-info {
  min-width: 0;
  flex: 1;
}

.setting-label {
  margin-bottom: 3px;
  font-size: 13.5px;
  font-weight: 600;
}

.setting-desc {
  margin: 0;
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.5;
}

.hotkey-input {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  gap: 5px;
  min-width: 146px;
  min-height: 36px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  cursor: pointer;
  outline: none;
  transition: border-color 0.16s ease, box-shadow 0.16s ease, background 0.16s ease,
    transform 0.12s ease;
}

.hotkey-input:active {
  transform: scale(0.99);
}

.hotkey-input:hover {
  border-color: var(--accent);
}

.hotkey-input:focus-visible {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.hotkey-input.recording {
  border-color: var(--accent);
  background: var(--accent-soft);
}

kbd {
  display: inline-flex;
  min-width: 22px;
  height: 24px;
  align-items: center;
  justify-content: center;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-bottom-width: 2px;
  border-radius: 5px;
  background: var(--card);
  color: var(--text);
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Consolas, monospace;
  font-size: 11.5px;
  line-height: 1;
}

.key-plus {
  color: var(--muted);
  font-size: 11px;
}

.recording-label {
  color: var(--accent);
  font-size: 12.5px;
  font-weight: 500;
  white-space: nowrap;
}

.error-banner {
  max-width: 720px;
  margin-bottom: 16px;
  padding: 11px 14px;
  border: 1px solid color-mix(in srgb, var(--danger) 25%, transparent);
  border-radius: 10px;
  background: var(--danger-bg);
  color: var(--danger);
  font-size: 12.5px;
  line-height: 1.5;
}

.switch {
  position: relative;
  display: inline-block;
  flex: 0 0 42px;
  width: 42px;
  height: 24px;
}

.switch input {
  width: 0;
  height: 0;
  opacity: 0;
}

.slider {
  position: absolute;
  inset: 0;
  border-radius: 24px;
  background: #c8c8c8;
  cursor: pointer;
  transition: background 0.2s cubic-bezier(0.2, 0.8, 0.2, 1), box-shadow 0.18s ease;
}

.slider::before {
  position: absolute;
  bottom: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #ffffff;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  content: "";
  transition: transform 0.22s cubic-bezier(0.2, 0.9, 0.25, 1);
}

.switch input:focus-visible + .slider {
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.switch input:checked + .slider {
  background: var(--accent);
}

.switch input:checked + .slider::before {
  transform: translateX(18px);
}

.page-footer {
  display: flex;
  max-width: 720px;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px 20px;
  margin-top: 30px;
  padding-top: 14px;
  border-top: 1px solid var(--border);
  color: var(--muted);
  font-size: 11.5px;
}

@keyframes fade-up {
  from {
    opacity: 0;
    transform: translateY(5px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes card-in {
  from {
    opacity: 0;
    transform: translateY(7px) scale(0.996);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

@media (prefers-reduced-motion: reduce) {
  .page-header,
  .settings-card {
    animation: none;
  }

  .nav-item,
  .hotkey-input,
  .slider,
  .slider::before,
  .setting-row,
  .settings-card {
    transition: none;
  }
}

@media (max-width: 720px) {
  .sidebar {
    flex-basis: 184px;
  }

  .content {
    padding: 28px 24px 22px;
  }

  .setting-row {
    gap: 18px;
    padding: 16px;
  }

  .hotkey-input {
    min-width: 128px;
  }
}
</style>
