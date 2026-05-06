<template>
  <div class="settings-container">
    <aside class="sidebar">
      <nav>
        <div class="nav-item active">快捷键</div>
      </nav>
    </aside>
    <main class="content">
      <h2>快捷键</h2>
      <div class="setting-group">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">截图</div>
            <div class="setting-desc">按下快捷键开始区域截图</div>
          </div>
          <div
            class="hotkey-input"
            :class="{ recording: isRecording }"
            tabindex="0"
            @click="startRecording"
            @keydown="onKeyDown"
            @blur="stopRecording"
          >
            {{ isRecording ? "按下快捷键..." : screenshotHotkey }}
          </div>
        </div>

        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">双击防误触</div>
            <div class="setting-desc">需要快速连按两次截图快捷键才会触发</div>
          </div>
          <label class="switch">
            <input type="checkbox" v-model="doublePress" @change="onToggle" />
            <span class="slider"></span>
          </label>
        </div>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

const doublePress = ref(false);
const screenshotHotkey = ref("F1");
const isRecording = ref(false);

onMounted(async () => {
  doublePress.value = await invoke("get_double_press_enabled");
  screenshotHotkey.value = await invoke("get_screenshot_hotkey");
});

function onToggle() {
  invoke("set_double_press_enabled", { enabled: doublePress.value });
}

function startRecording() {
  isRecording.value = true;
}

function stopRecording() {
  isRecording.value = false;
}

function onKeyDown(e: KeyboardEvent) {
  if (!isRecording.value) return;
  e.preventDefault();
  e.stopPropagation();

  const modifiers: string[] = [];
  if (e.ctrlKey) modifiers.push("Ctrl");
  if (e.altKey) modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey) modifiers.push("Super");

  let key = e.key;
  if (
    key === "Control" ||
    key === "Alt" ||
    key === "Shift" ||
    key === "Meta"
  ) {
    return;
  }

  if (key === " ") key = "Space";
  if (key.length === 1) key = key.toUpperCase();

  const parts = [...modifiers, key];
  const hotkeyStr = parts.join("+");

  screenshotHotkey.value = hotkeyStr;
  isRecording.value = false;

  invoke("set_screenshot_hotkey", { hotkey: hotkeyStr }).catch((err) => {
    console.error("Failed to set hotkey:", err);
    invoke("get_screenshot_hotkey").then((v) => {
      screenshotHotkey.value = v as string;
    });
  });
}
</script>

<style scoped>
.settings-container {
  display: flex;
  width: 100vw;
  height: 100vh;
  background: #ffffff;
  color: #1d1d1f;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}

.sidebar {
  width: 200px;
  background: #f5f5f7;
  border-right: 1px solid #e5e5e5;
  padding: 24px 0;
  flex-shrink: 0;
}

.nav-item {
  padding: 10px 20px;
  font-size: 14px;
  color: #666;
  cursor: pointer;
  border-left: 3px solid transparent;
  transition: all 0.15s;
}

.nav-item.active,
.nav-item:hover {
  color: #1d1d1f;
  background: rgba(0, 0, 0, 0.03);
}

.nav-item.active {
  border-left-color: #007aff;
  background: rgba(0, 122, 255, 0.06);
  font-weight: 500;
}

.content {
  flex: 1;
  padding: 32px 40px;
  overflow-y: auto;
}

h2 {
  font-size: 22px;
  font-weight: 600;
  margin-bottom: 24px;
  color: #1d1d1f;
}

.setting-group {
  max-width: 600px;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 0;
  border-bottom: 1px solid #f0f0f0;
}

.setting-info {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.setting-label {
  font-size: 15px;
  font-weight: 500;
  color: #1d1d1f;
}

.setting-desc {
  font-size: 13px;
  color: #86868b;
}

.hotkey-input {
  min-width: 120px;
  padding: 8px 14px;
  border: 1px solid #d1d1d6;
  border-radius: 8px;
  font-size: 14px;
  font-family: ui-monospace, SFMono-Regular, "SF Mono", monospace;
  text-align: center;
  color: #1d1d1f;
  background: #fafafa;
  cursor: pointer;
  transition: all 0.15s;
  outline: none;
  user-select: none;
}

.hotkey-input:hover {
  border-color: #007aff;
  background: #fff;
}

.hotkey-input.recording {
  border-color: #007aff;
  background: rgba(0, 122, 255, 0.05);
  color: #007aff;
  animation: pulse 1.5s infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.6;
  }
}

/* Toggle switch */
.switch {
  position: relative;
  display: inline-block;
  width: 44px;
  height: 24px;
}

.switch input {
  opacity: 0;
  width: 0;
  height: 0;
}

.slider {
  position: absolute;
  cursor: pointer;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background-color: #d1d1d6;
  transition: 0.2s;
  border-radius: 24px;
}

.slider:before {
  position: absolute;
  content: "";
  height: 18px;
  width: 18px;
  left: 3px;
  bottom: 3px;
  background-color: white;
  transition: 0.2s;
  border-radius: 50%;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
}

input:checked + .slider {
  background-color: #007aff;
}

input:checked + .slider:before {
  transform: translateX(20px);
}
</style>
