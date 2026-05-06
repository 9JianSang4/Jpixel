<template>
  <div class="capture-layer" @mousedown="onMouseDown" @mousemove="onMouseMove" @mouseup="onMouseUp" @dblclick="onConfirm" tabindex="0" ref="layerRef">
    <!-- Crosshair cursor lines -->
    <div v-if="!isDragging && !hasSelection" class="crosshair-h" :style="{ top: cursorY + 'px' }" />
    <div v-if="!isDragging && !hasSelection" class="crosshair-v" :style="{ left: cursorX + 'px' }" />

    <!-- Dark overlay with hole -->
    <template v-if="hasSelection">
      <div class="overlay top" :style="{ height: selTop + 'px' }" />
      <div class="overlay bottom" :style="{ top: selBottom + 'px' }" />
      <div class="overlay left" :style="{ top: selTop + 'px', width: selLeft + 'px', height: selHeight + 'px' }" />
      <div class="overlay right" :style="{ top: selTop + 'px', left: selRight + 'px', height: selHeight + 'px' }" />
    </template>
    <div v-else class="overlay full" />

    <!-- Selection border -->
    <div v-if="hasSelection" class="selection-box" :style="selectionStyle">
      <!-- Size label -->
      <div class="size-label" :style="{ top: selHeight > 40 ? '-24px' : '4px' }">
        {{ selWidth }} x {{ selHeight }}
      </div>
    </div>

    <!-- Floating toolbar -->
    <div v-if="hasSelection" class="toolbar" :style="toolbarStyle" @mousedown.stop @mouseup.stop>
      <button class="tool-btn" @click.stop="onConfirm" title="确认 (Enter/双击)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click.stop="onCancel" title="取消 (Esc)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

const layerRef = ref<HTMLDivElement | null>(null);
const cursorX = ref(0);
const cursorY = ref(0);

const startX = ref(0);
const startY = ref(0);
const endX = ref(0);
const endY = ref(0);
const isDragging = ref(false);
const hasSelection = ref(false);
const dragMode = ref<'create' | 'move'>('create');
const moveOffsetX = ref(0);
const moveOffsetY = ref(0);

const selLeft = computed(() => Math.min(startX.value, endX.value));
const selTop = computed(() => Math.min(startY.value, endY.value));
const selRight = computed(() => Math.max(startX.value, endX.value));
const selBottom = computed(() => Math.max(startY.value, endY.value));
const selWidth = computed(() => selRight.value - selLeft.value);
const selHeight = computed(() => selBottom.value - selTop.value);

const selectionStyle = computed(() => ({
  left: selLeft.value + "px",
  top: selTop.value + "px",
  width: selWidth.value + "px",
  height: selHeight.value + "px",
}));

const toolbarStyle = computed(() => {
  const top = selBottom.value + 12;
  const left = selLeft.value;
  return {
    left: left + "px",
    top: top + "px",
  };
});

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;

  if (hasSelection.value) {
    const inSelection =
      e.clientX >= selLeft.value &&
      e.clientX <= selRight.value &&
      e.clientY >= selTop.value &&
      e.clientY <= selBottom.value;
    if (inSelection) {
      dragMode.value = 'move';
      moveOffsetX.value = e.clientX - selLeft.value;
      moveOffsetY.value = e.clientY - selTop.value;
      isDragging.value = true;
      return;
    }
  }

  dragMode.value = 'create';
  isDragging.value = true;
  hasSelection.value = true;
  startX.value = e.clientX;
  startY.value = e.clientY;
  endX.value = e.clientX;
  endY.value = e.clientY;
}

function onMouseMove(e: MouseEvent) {
  cursorX.value = e.clientX;
  cursorY.value = e.clientY;
  if (!isDragging.value) return;

  if (dragMode.value === 'move') {
    const newLeft = e.clientX - moveOffsetX.value;
    const newTop = e.clientY - moveOffsetY.value;
    const w = selWidth.value;
    const h = selHeight.value;
    startX.value = newLeft;
    startY.value = newTop;
    endX.value = newLeft + w;
    endY.value = newTop + h;
  } else {
    endX.value = e.clientX;
    endY.value = e.clientY;
  }
}

function onMouseUp() {
  isDragging.value = false;
}

async function onConfirm() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) {
    onCancel();
    return;
  }
  try {
    const path: string = await invoke("capture_screen_region", {
      x: selLeft.value,
      y: selTop.value,
      width: selWidth.value,
      height: selHeight.value,
    });
    await invoke("close_capture_windows");
    await invoke("create_editor_window", { imagePath: path });
  } catch (e) {
    console.error("Capture failed:", e);
    onCancel();
  }
}

function onCancel() {
  invoke("close_capture_windows");
}

function onKeyDown(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    onConfirm();
  }
  if (e.key === "Escape") {
    e.preventDefault();
    onCancel();
  }
}

onMounted(() => {
  layerRef.value?.focus();
  window.addEventListener("keydown", onKeyDown);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
});
</script>

<style scoped>
.capture-layer {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  cursor: crosshair;
  outline: none;
  overflow: hidden;
}

.crosshair-h {
  position: fixed;
  left: 0;
  width: 100vw;
  height: 1px;
  background: rgba(255, 255, 255, 0.4);
  pointer-events: none;
  z-index: 10;
}

.crosshair-v {
  position: fixed;
  top: 0;
  width: 1px;
  height: 100vh;
  background: rgba(255, 255, 255, 0.4);
  pointer-events: none;
  z-index: 10;
}

.overlay {
  position: fixed;
  left: 0;
  background: rgba(0, 0, 0, 0.45);
  pointer-events: none;
}

.overlay.full {
  top: 0;
  width: 100vw;
  height: 100vh;
}

.overlay.top {
  top: 0;
  width: 100vw;
}

.overlay.bottom {
  width: 100vw;
  bottom: 0;
}

.overlay.left {
  left: 0;
}

.overlay.right {
  right: 0;
  width: auto;
}

.selection-box {
  position: fixed;
  border: 1.5px solid rgba(100, 181, 246, 0.9);
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.2);
  pointer-events: none;
  z-index: 5;
}

.size-label {
  position: absolute;
  left: 0;
  padding: 2px 6px;
  background: rgba(100, 181, 246, 0.9);
  color: #fff;
  font-size: 11px;
  font-family: monospace;
  border-radius: 3px;
  pointer-events: none;
  white-space: nowrap;
}

.toolbar {
  position: fixed;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 8px;
  border-radius: 10px;
  background: rgba(30, 30, 40, 0.65);
  backdrop-filter: blur(16px) saturate(180%);
  -webkit-backdrop-filter: blur(16px) saturate(180%);
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
  z-index: 20;
  pointer-events: auto;
}

.tool-btn {
  width: 30px;
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: rgba(255, 255, 255, 0.85);
  cursor: pointer;
  transition: background 0.15s;
}

.tool-btn:hover {
  background: rgba(255, 255, 255, 0.12);
}

.divider {
  width: 1px;
  height: 18px;
  background: rgba(255, 255, 255, 0.15);
}
</style>
