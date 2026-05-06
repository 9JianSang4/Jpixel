<template>
  <div class="settings-container">
    <div class="glass-panel">
      <h1>Jpixel Settings</h1>

      <div class="setting-item">
        <label>快捷键</label>
        <div class="value">F1</div>
      </div>

      <div class="setting-item">
        <label>双击防误触</label>
        <label class="switch">
          <input type="checkbox" v-model="doublePress" @change="onToggle" />
          <span class="slider"></span>
        </label>
      </div>

      <p class="hint">
        开启后，需要快速连按两次 F1 才会触发截图，防止游戏中误触。
      </p>

      <button class="capture-btn" @click="triggerCapture">立即截图</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

const doublePress = ref(false);

onMounted(async () => {
  doublePress.value = await invoke("get_double_press_enabled");
});

function onToggle() {
  invoke("set_double_press_enabled", { enabled: doublePress.value });
}

function triggerCapture() {
  invoke("create_capture_window");
}
</script>

<style scoped>
.settings-container {
  width: 100vw;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
}

.glass-panel {
  width: 400px;
  padding: 32px;
  border-radius: 20px;
  background: rgba(255, 255, 255, 0.08);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

h1 {
  color: #fff;
  font-size: 24px;
  font-weight: 600;
  margin-bottom: 24px;
  text-align: center;
}

.setting-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.setting-item label {
  color: rgba(255, 255, 255, 0.85);
  font-size: 15px;
}

.value {
  color: #64b5f6;
  font-family: monospace;
  font-size: 14px;
  background: rgba(100, 181, 246, 0.1);
  padding: 4px 10px;
  border-radius: 6px;
}

.hint {
  color: rgba(255, 255, 255, 0.5);
  font-size: 13px;
  margin-top: 12px;
  line-height: 1.5;
}

.capture-btn {
  width: 100%;
  margin-top: 24px;
  padding: 12px;
  border: none;
  border-radius: 10px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
  font-size: 15px;
  font-weight: 500;
  cursor: pointer;
  transition: transform 0.15s, opacity 0.15s;
}

.capture-btn:hover {
  opacity: 0.9;
  transform: translateY(-1px);
}

.capture-btn:active {
  transform: translateY(0);
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
  background-color: rgba(255, 255, 255, 0.2);
  transition: 0.3s;
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
  transition: 0.3s;
  border-radius: 50%;
}

input:checked + .slider {
  background-color: #667eea;
}

input:checked + .slider:before {
  transform: translateX(20px);
}
</style>
