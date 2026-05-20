<template>
  <div class="editor-container" @keydown="onKeyDown" tabindex="0" ref="editorRef">
    <!-- Toolbar -->
    <div class="toolbar">
      <button
        v-for="t in tools"
        :key="t.id"
        class="tool-btn"
        :class="{ active: currentTool === t.id }"
        @click="selectTool(t.id)"
        :title="t.label"
      >
        <span>{{ t.icon }}</span>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click="undo" title="撤销 (Ctrl+Z)">
        <span>↩</span>
      </button>
      <button class="tool-btn" @click="clearAll" title="清空">
        <span>🗑</span>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click="saveImage" title="保存">
        <span>💾</span>
      </button>
      <button class="tool-btn" @click="copyImage" title="复制">
        <span>📋</span>
      </button>
    </div>

    <!-- Canvas area -->
    <div class="canvas-wrapper" ref="wrapperRef">
      <img
        v-if="imageSrc"
        :src="imageSrc"
        ref="imgRef"
        draggable="false"
        @load="onImageLoad"
      />
      <canvas
        v-if="imageSrc"
        ref="canvasRef"
        @mousedown="onMouseDown"
        @mousemove="onMouseMove"
        @mouseup="onMouseUp"
        @click="onCanvasClick"
      />
      <div v-else class="placeholder">Loading image...</div>
    </div>

    <!-- Text input overlay -->
    <div
      v-if="textInput.visible"
      class="text-input-overlay"
      :style="{
        left: textInput.x + 'px',
        top: textInput.y + 'px',
      }"
    >
      <input
        ref="textInputRef"
        v-model="textInput.value"
        @keydown.enter="commitText"
        @blur="commitText"
        :style="{ color: strokeColor, fontSize: textSize + 'px' }"
      />
    </div>

    <!-- Color picker -->
    <div class="color-bar">
      <button
        v-for="c in colors"
        :key="c"
        class="color-btn"
        :class="{ active: strokeColor === c }"
        :style="{ background: c }"
        @click="strokeColor = c"
      />
      <div class="divider" />
      <label>粗细</label>
      <input type="range" min="1" max="10" v-model.number="strokeWidth" />
      <div class="divider" />
      <label>字号</label>
      <input type="range" min="12" max="48" v-model.number="textSize" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import {
  readImageBase64,
} from "../api/ipc";

// ─────────────────────────────────────────────────────────────
// Refs
// ─────────────────────────────────────────────────────────────

const editorRef = ref<HTMLDivElement | null>(null);
const wrapperRef = ref<HTMLDivElement | null>(null);
const canvasRef = ref<HTMLCanvasElement | null>(null);
const imgRef = ref<HTMLImageElement | null>(null);
const textInputRef = ref<HTMLInputElement | null>(null);

const imageSrc = ref("");
const imagePath = ref("");
const route = useRoute();

// ─────────────────────────────────────────────────────────────
// Tool state
// ─────────────────────────────────────────────────────────────

const tools = [
  { id: "rect", label: "矩形", icon: "□" },
  { id: "arrow", label: "箭头", icon: "→" },
  { id: "text", label: "文字", icon: "T" },
  { id: "mosaic", label: "马赛克", icon: "▦" },
] as const;

type ToolId = (typeof tools)[number]["id"];

const currentTool = ref<ToolId | null>(null);
const strokeColor = ref("#FF0000");
const strokeWidth = ref(2);
const textSize = ref(20);

const colors = ["#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FF00FF", "#00FFFF", "#FFFFFF", "#000000"];

// ─────────────────────────────────────────────────────────────
// Drawing state
// ─────────────────────────────────────────────────────────────

interface DrawAction {
  tool: ToolId;
  x: number;
  y: number;
  width?: number;
  height?: number;
  endX?: number;
  endY?: number;
  color: string;
  strokeWidth?: number;
  text?: string;
  fontSize?: number;
}

let isDrawing = false;
let startX = 0;
let startY = 0;
const actions = ref<DrawAction[]>([]);

// ─────────────────────────────────────────────────────────────
// Text input
// ─────────────────────────────────────────────────────────────

const textInput = ref({ visible: false, x: 0, y: 0, value: "" });

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

function getCanvasCoords(e: MouseEvent) {
  const canvas = canvasRef.value!;
  const rect = canvas.getBoundingClientRect();
  const scaleX = canvas.width / rect.width;
  const scaleY = canvas.height / rect.height;
  return {
    x: (e.clientX - rect.left) * scaleX,
    y: (e.clientY - rect.top) * scaleY,
  };
}

function redraw() {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  ctx.clearRect(0, 0, canvas.width, canvas.height);

  for (const action of actions.value) {
    ctx.strokeStyle = action.color;
    ctx.fillStyle = action.color;
    ctx.lineWidth = action.strokeWidth || 2;

    switch (action.tool) {
      case "rect":
        if (action.width && action.height) {
          ctx.strokeRect(action.x, action.y, action.width, action.height);
        }
        break;
      case "arrow":
        if (action.endX !== undefined && action.endY !== undefined) {
          drawArrow(ctx, action.x, action.y, action.endX, action.endY);
        }
        break;
      case "text":
        if (action.text) {
          ctx.font = `${action.fontSize || 20}px sans-serif`;
          ctx.fillStyle = action.color;
          ctx.fillText(action.text, action.x, action.y);
        }
        break;
      case "mosaic":
        if (action.width && action.height) {
          drawMosaic(ctx, action.x, action.y, action.width, action.height);
        }
        break;
    }
  }
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number
) {
  const headLen = 12;
  const angle = Math.atan2(y2 - y1, x2 - x1);

  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(
    x2 - headLen * Math.cos(angle - Math.PI / 6),
    y2 - headLen * Math.sin(angle - Math.PI / 6)
  );
  ctx.lineTo(
    x2 - headLen * Math.cos(angle + Math.PI / 6),
    y2 - headLen * Math.sin(angle + Math.PI / 6)
  );
  ctx.lineTo(x2, y2);
  ctx.fill();
}

function drawMosaic(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number
) {
  const BLOCK_SIZE = 8;
  ctx.save();
  for (let iy = y; iy < y + h; iy += BLOCK_SIZE) {
    for (let ix = x; ix < x + w; ix += BLOCK_SIZE) {
      const bh = Math.min(BLOCK_SIZE, y + h - iy);
      const bw = Math.min(BLOCK_SIZE, x + w - ix);
      // Deterministic pseudo-random color based on block position
      // so redraws don't flicker.
      const seed = (ix * 73856093) ^ (iy * 19349663);
      const r = 100 + (Math.abs(seed) % 50);
      const g = 100 + (Math.abs(seed * 7) % 50);
      const b = 100 + (Math.abs(seed * 13) % 50);
      ctx.fillStyle = `rgba(${r},${g},${b},0.8)`;
      ctx.fillRect(ix, iy, bw, bh);
    }
  }
  ctx.restore();
}

// ─────────────────────────────────────────────────────────────
// Event handlers
// ─────────────────────────────────────────────────────────────

function selectTool(tool: ToolId) {
  if (currentTool.value === tool) {
    currentTool.value = null;
  } else {
    currentTool.value = tool;
  }
}

function onImageLoad() {
  const img = imgRef.value;
  const canvas = canvasRef.value;
  if (!img || !canvas) return;

  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
}

function onMouseDown(e: MouseEvent) {
  if (!currentTool.value || currentTool.value === "text") return;
  const pos = getCanvasCoords(e);
  isDrawing = true;
  startX = pos.x;
  startY = pos.y;
}

function onMouseMove(e: MouseEvent) {
  if (!isDrawing || !currentTool.value) return;
  const pos = getCanvasCoords(e);

  redraw();

  const ctx = canvasRef.value!.getContext("2d")!;
  ctx.strokeStyle = strokeColor.value;
  ctx.fillStyle = strokeColor.value;
  ctx.lineWidth = strokeWidth.value;

  const x = Math.min(startX, pos.x);
  const y = Math.min(startY, pos.y);
  const w = Math.abs(pos.x - startX);
  const h = Math.abs(pos.y - startY);

  switch (currentTool.value) {
    case "rect":
      ctx.strokeRect(x, y, w, h);
      break;
    case "arrow":
      drawArrow(ctx, startX, startY, pos.x, pos.y);
      break;
    case "mosaic":
      drawMosaic(ctx, x, y, w, h);
      break;
  }
}

function onMouseUp(e: MouseEvent) {
  if (!isDrawing || !currentTool.value) return;
  isDrawing = false;
  const pos = getCanvasCoords(e);

  const x = Math.min(startX, pos.x);
  const y = Math.min(startY, pos.y);
  const w = Math.abs(pos.x - startX);
  const h = Math.abs(pos.y - startY);

  if (w < 2 && h < 2) {
    redraw();
    return;
  }

  actions.value.push({
    tool: currentTool.value,
    x,
    y,
    width: w,
    height: h,
    endX: pos.x,
    endY: pos.y,
    color: strokeColor.value,
    strokeWidth: strokeWidth.value,
  });

  redraw();
}

function onCanvasClick(e: MouseEvent) {
  if (currentTool.value !== "text") return;
  const pos = getCanvasCoords(e);
  textInput.value = {
    visible: true,
    x: e.clientX,
    y: e.clientY,
    value: "",
  };
  startX = pos.x;
  startY = pos.y;
  setTimeout(() => textInputRef.value?.focus(), 0);
}

function commitText() {
  if (!textInput.value.visible) return;
  const text = textInput.value.value.trim();
  if (text) {
    actions.value.push({
      tool: "text",
      x: startX,
      y: startY + textSize.value,
      color: strokeColor.value,
      text,
      fontSize: textSize.value,
    });
    redraw();
  }
  textInput.value.visible = false;
  textInput.value.value = "";
}

// ─────────────────────────────────────────────────────────────
// Actions
// ─────────────────────────────────────────────────────────────

function undo() {
  actions.value.pop();
  redraw();
}

function clearAll() {
  actions.value = [];
  redraw();
}

function getMergedImage(): string | null {
  const canvas = document.createElement("canvas");
  const img = imgRef.value;
  if (!img) return null;

  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext("2d")!;
  ctx.drawImage(img, 0, 0);

  const mainCtx = canvasRef.value?.getContext("2d");
  if (mainCtx && canvasRef.value) {
    ctx.drawImage(canvasRef.value, 0, 0);
  }

  return canvas.toDataURL("image/png");
}

async function saveImage() {
  const dataUrl = getMergedImage();
  if (!dataUrl) return;

  // Convert data URL to blob and trigger download
  const response = await fetch(dataUrl);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);

  const a = document.createElement("a");
  a.href = url;
  a.download = `jpixel-edit-${Date.now()}.png`;
  a.click();
  URL.revokeObjectURL(url);
}

async function copyImage() {
  try {
    const dataUrl = getMergedImage();
    if (!dataUrl) return;

    const response = await fetch(dataUrl);
    const blob = await response.blob();
    await navigator.clipboard.write([
      new ClipboardItem({ "image/png": blob }),
    ]);
  } catch (e) {
    console.error("Copy image failed:", e);
    alert("复制图片失败，请检查浏览器剪贴板权限。");
  }
}

function onKeyDown(e: KeyboardEvent) {
  if (e.ctrlKey && e.key.toLowerCase() === "z") {
    e.preventDefault();
    undo();
  }
  if (e.key === "Escape") {
    currentTool.value = null;
  }
}

// ─────────────────────────────────────────────────────────────
// Lifecycle
// ─────────────────────────────────────────────────────────────

onMounted(async () => {
  editorRef.value?.focus();
  const path = route.query.path as string | undefined;
  if (path) {
    imagePath.value = path;
    try {
      imageSrc.value = await readImageBase64(path);
    } catch (e) {
      console.error("Failed to load image:", e);
    }
  }
});

onUnmounted(() => {
  // cleanup
});
</script>

<style scoped>
.editor-container {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: #1a1a1f;
  outline: none;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 12px;
  background: #25252b;
  border-bottom: 1px solid #333;
  flex-shrink: 0;
}

.tool-btn {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: #ccc;
  cursor: pointer;
  font-size: 16px;
  transition: all 0.15s;
}

.tool-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.tool-btn.active {
  background: #007aff;
  color: #fff;
}

.divider {
  width: 1px;
  height: 20px;
  background: #444;
  margin: 0 4px;
}

.canvas-wrapper {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: auto;
  position: relative;
}

.canvas-wrapper img {
  max-width: 95vw;
  max-height: 85vh;
  object-fit: contain;
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.5);
  border-radius: 4px;
  user-select: none;
}

.canvas-wrapper canvas {
  position: absolute;
  max-width: 95vw;
  max-height: 85vh;
  pointer-events: auto;
  cursor: crosshair;
}

.placeholder {
  color: rgba(255, 255, 255, 0.6);
  font-size: 16px;
}

.color-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  background: #25252b;
  border-top: 1px solid #333;
  flex-shrink: 0;
}

.color-bar label {
  color: #aaa;
  font-size: 12px;
}

.color-btn {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
  padding: 0;
}

.color-btn.active {
  border-color: #fff;
}

.text-input-overlay {
  position: fixed;
  z-index: 100;
}

.text-input-overlay input {
  background: transparent;
  border: 1px dashed #fff;
  outline: none;
  padding: 2px 4px;
  font-family: sans-serif;
  min-width: 60px;
}
</style>
