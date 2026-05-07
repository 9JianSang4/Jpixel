<template>
  <div
    class="pin-container"
    data-tauri-drag-region
    @wheel="onWheel"
    @contextmenu.prevent="onClose"
    @keydown="onKeyDown"
    tabindex="0"
    ref="containerRef"
  >
    <div class="img-wrapper" v-if="imageSrc && !loadError">
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
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";

const containerRef = ref<HTMLDivElement | null>(null);
const imgRef = ref<HTMLImageElement | null>(null);
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
  const img = imgRef.value;
  if (!img) return;

  // screenshots crate captures physical pixels; convert to logical size
  const dpr = window.devicePixelRatio || 1;
  let w = img.naturalWidth / dpr;
  let h = img.naturalHeight / dpr;

  const maxW = window.screen.width * 0.8;
  const maxH = window.screen.height * 0.8;

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

  baseWidth.value = w;
  baseHeight.value = h;
  scale.value = 1.0;

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

  const win = getCurrentWebviewWindow();
  const newW = baseWidth.value * newScale;
  const newH = baseHeight.value * newScale;
  win.setSize(new LogicalSize(newW, newH)).catch(console.error);
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
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.img-wrapper img {
  display: block;
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
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
