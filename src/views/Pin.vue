<template>
  <div
    class="pin-container"
    data-tauri-drag-region
    @contextmenu.prevent="onClose"
    @keydown="onKeyDown"
    @wheel.prevent="onWindowWheel"
    @mousedown.left="onMouseDown"
    tabindex="0"
    ref="containerRef"
  >
    <div class="img-wrapper" v-if="imageSrc && !loadError">
      <img
        :src="imageSrc"
        draggable="false"
        @load="onImageLoad"
        @error="onImageError"
        ref="imgRef"
      />
    </div>
    <div v-else-if="loadError" class="placeholder error">图片加载失败，右键关闭</div>
    <div v-else class="placeholder">贴图窗口</div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";

const containerRef = ref<HTMLDivElement | null>(null);
const imageSrc = ref("");
const loadError = ref(false);
const baseWidth = ref(0);
const baseHeight = ref(0);
const scale = ref(1.0);

const SCALE_MIN = 0.5;
const SCALE_MAX = 3.0;
const SCALE_STEP = 0.1;

const route = useRoute();

onMounted(async () => {
  containerRef.value?.focus();

  let w = parseInt(route.query.width as string) || 400;
  let h = parseInt(route.query.height as string) || 300;
  w = Math.max(10, Math.min(8000, w));
  h = Math.max(10, Math.min(8000, h));
  baseWidth.value = w;
  baseHeight.value = h;

  const path = route.query.path as string | undefined;
  if (path) {
    try {
      const base64: string = await invoke("read_image_base64", { path });
      imageSrc.value = base64;
    } catch (e) {
      loadError.value = true;
      console.error("[Pin] Failed to load image:", e);
    }
  }
});

function onImageLoad() {
  // Window size is already set by Rust using the selection dimensions.
}

function onImageError() {
  loadError.value = true;
  console.error("[Pin] Failed to load image:", imageSrc.value);
}

function onWindowWheel(e: WheelEvent) {
  const delta = e.deltaY > 0 ? -SCALE_STEP : SCALE_STEP;
  let newScale = scale.value + delta;
  newScale = Math.max(SCALE_MIN, Math.min(SCALE_MAX, newScale));
  scale.value = Math.round(newScale * 100) / 100;

  const win = getCurrentWebviewWindow();
  const newW = Math.round(baseWidth.value * newScale);
  const newH = Math.round(baseHeight.value * newScale);
  win.setSize(new LogicalSize(newW, newH)).catch((err: unknown) => {
    console.error("[Pin] setSize failed:", err);
  });
}

function onMouseDown(e: MouseEvent) {
  // data-tauri-drag-region handles most cases; this is a fallback.
  e.preventDefault();
  getCurrentWebviewWindow().startDragging().catch((err: unknown) => {
    console.error("[Pin] startDragging failed:", err);
  });
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
  background: #000;
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
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.img-wrapper img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: fill;
  user-select: none;
  -webkit-user-drag: none;
  border: 1px solid rgba(255, 255, 255, 0.25);
  box-sizing: border-box;
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
