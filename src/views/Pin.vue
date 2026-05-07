<template>
  <div
    class="pin-container"
    @wheel="onWheel"
    @contextmenu.prevent="onClose"
    @keydown="onKeyDown"
    tabindex="0"
    ref="containerRef"
  >
    <div
      class="img-wrapper"
      :style="{ transform: `scale(${scale})` }"
      v-if="imageSrc && !loadError"
      data-tauri-drag-region
    >
      <img :src="imageSrc" draggable="false" @load="onImageLoad" @error="onImageError" ref="imgRef" />
    </div>
    <div v-else-if="loadError" class="placeholder error">图片加载失败，右键关闭</div>
    <div v-else class="placeholder">贴图窗口</div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";

const containerRef = ref<HTMLDivElement | null>(null);
const imgRef = ref<HTMLImageElement | null>(null);
const imageSrc = ref("");
const scale = ref(1.0);
const loadError = ref(false);

const SCALE_MIN = 0.2;
const SCALE_MAX = 5.0;
const SCALE_STEP = 0.1;

const route = useRoute();

onMounted(() => {
  containerRef.value?.focus();
  const path = route.query.path as string | undefined;
  console.log("[Pin] path from route:", path);
  if (path) {
    imageSrc.value = convertFileSrc(path);
  }
});

function onImageLoad() {
  const img = imgRef.value;
  if (!img) return;

  const naturalW = img.naturalWidth;
  const naturalH = img.naturalHeight;

  const maxW = window.screen.width * 0.8;
  const maxH = window.screen.height * 0.8;

  let w = naturalW;
  let h = naturalH;

  if (w > maxW) {
    const ratio = maxW / w;
    w = maxW;
    h = h * ratio;
  }
  if (h > maxH) {
    const ratio = maxH / h;
    h = maxH;
    w = w * ratio;
  }

  const win = getCurrentWebviewWindow();
  win.setSize(new LogicalSize(w, h)).catch(console.error);
}

function onImageError() {
  loadError.value = true;
  console.error("[Pin] Failed to load image:", imageSrc.value);
}

function onWheel(e: WheelEvent) {
  e.preventDefault();
  const delta = e.deltaY > 0 ? -SCALE_STEP : SCALE_STEP;
  let newScale = scale.value + delta;
  newScale = Math.max(SCALE_MIN, Math.min(SCALE_MAX, newScale));
  scale.value = parseFloat(newScale.toFixed(2));
}

function onClose() {
  const win = getCurrentWebviewWindow();
  invoke("close_pin_window", { label: win.label }).catch(() => {
    win.close().catch(() => {});
  });
}

function onKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    onClose();
  }
}
</script>

<style scoped>
.pin-container {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background: transparent;
  outline: none;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: grab;
}

.pin-container:active {
  cursor: grabbing;
}

.img-wrapper {
  transform-origin: center center;
  transition: transform 0.05s linear;
  will-change: transform;
  pointer-events: none;
}

.img-wrapper img {
  display: block;
  max-width: none;
  max-height: none;
  user-select: none;
  -webkit-user-drag: none;
}

.placeholder {
  color: #fff;
  font-size: 14px;
  opacity: 0.5;
}

.placeholder.error {
  color: #ff6b6b;
  opacity: 1;
}
</style>
