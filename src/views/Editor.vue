<template>
  <div class="editor-container" @keydown="onKeyDown" tabindex="0" ref="editorRef">
    <!-- Toolbar -->
    <div class="toolbar">
      <button
        class="tool-btn"
        :class="{ active: currentTool === 'pen' }"
        @click="selectTool('pen')"
        title="画笔"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 19l7-7 3 3-7 7-3-3z"/><path d="M18 13l-1.5-7.5L2 2l3.5 14.5L13 18l5-5z"/><path d="M2 2l7.586 7.586"/><circle cx="11" cy="11" r="2"/>
        </svg>
      </button>
      <button
        class="tool-btn"
        :class="{ active: currentTool === 'eraser' }"
        @click="selectTool('eraser')"
        title="擦除"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M20 20H7L3 16c-.8-.8-.8-2 0-2.8l9.2-9.2c.8-.8 2-.8 2.8 0L20 8.8c.8.8.8 2 0 2.8L11 20"/>
        </svg>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click="undo" title="撤销 (Ctrl+Z)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/>
        </svg>
      </button>
      <button class="tool-btn" @click="clearAll" title="清空">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
        </svg>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click="saveImage" title="保存">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/>
        </svg>
      </button>
      <button class="tool-btn" @click="copyImage" title="复制">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
        </svg>
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
        @mouseleave="onMouseUp"
      />
      <div v-else class="placeholder">加载中...</div>
    </div>

    <!-- Color / size bar -->
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
      <input type="range" min="1" max="20" v-model.number="strokeWidth" />
      <span class="size-label">{{ strokeWidth }}px</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute } from "vue-router";
import { readImageBase64 } from "../api/ipc";

// ─────────────────────────────────────────────────────────────
// Refs
// ─────────────────────────────────────────────────────────────

const editorRef = ref<HTMLDivElement | null>(null);
const wrapperRef = ref<HTMLDivElement | null>(null);
const canvasRef = ref<HTMLCanvasElement | null>(null);
const imgRef = ref<HTMLImageElement | null>(null);

const imageSrc = ref("");
const route = useRoute();

// ─────────────────────────────────────────────────────────────
// Tool state
// ─────────────────────────────────────────────────────────────

type ToolId = "pen" | "eraser";

const currentTool = ref<ToolId | null>(null);
const strokeColor = ref("#FF0000");
const strokeWidth = ref(3);

const colors = ["#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FF00FF", "#00FFFF", "#FFFFFF", "#000000"];

// ─────────────────────────────────────────────────────────────
// Drawing state
// ─────────────────────────────────────────────────────────────

interface Point {
  x: number;
  y: number;
}

interface PenAction {
  type: "pen";
  points: Point[];
  color: string;
  width: number;
}

interface EraserAction {
  type: "eraser";
  points: Point[];
  size: number;
}

type DrawAction = PenAction | EraserAction;

let isDrawing = false;
const actions = ref<DrawAction[]>([]);
let currentPoints: Point[] = [];

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
    if (action.type === "pen") {
      drawPenStroke(ctx, action.points, action.color, action.width);
    } else if (action.type === "eraser") {
      drawEraserStroke(ctx, action.points, action.size);
    }
  }

  // Draw in-progress stroke
  if (currentPoints.length > 1) {
    const tool = currentTool.value;
    if (tool === "pen") {
      drawPenStroke(ctx, currentPoints, strokeColor.value, strokeWidth.value);
    } else if (tool === "eraser") {
      drawEraserStroke(ctx, currentPoints, strokeWidth.value * 4);
    }
  }
}

function drawPenStroke(
  ctx: CanvasRenderingContext2D,
  points: Point[],
  color: string,
  width: number,
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.globalCompositeOperation = "source-over";

  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    ctx.lineTo(points[i].x, points[i].y);
  }
  ctx.stroke();
  ctx.restore();
}

function drawEraserStroke(
  ctx: CanvasRenderingContext2D,
  points: Point[],
  size: number,
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.strokeStyle = "rgba(0,0,0,1)";
  ctx.lineWidth = size;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.globalCompositeOperation = "destination-out";

  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    ctx.lineTo(points[i].x, points[i].y);
  }
  ctx.stroke();
  ctx.restore();
}

// ─────────────────────────────────────────────────────────────
// Event handlers
// ─────────────────────────────────────────────────────────────

function selectTool(tool: ToolId) {
  currentTool.value = currentTool.value === tool ? null : tool;
}

function onImageLoad() {
  const img = imgRef.value;
  const canvas = canvasRef.value;
  if (!img || !canvas) return;

  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
}

function onMouseDown(e: MouseEvent) {
  if (!currentTool.value) return;
  const pos = getCanvasCoords(e);
  isDrawing = true;
  currentPoints = [pos];
}

function onMouseMove(e: MouseEvent) {
  if (!isDrawing || !currentTool.value) return;
  const pos = getCanvasCoords(e);
  currentPoints.push(pos);
  redraw();
}

function onMouseUp() {
  if (!isDrawing || !currentTool.value) return;
  isDrawing = false;

  if (currentTool.value === "pen") {
    actions.value.push({
      type: "pen",
      points: [...currentPoints],
      color: strokeColor.value,
      width: strokeWidth.value,
    });
  } else if (currentTool.value === "eraser") {
    actions.value.push({
      type: "eraser",
      points: [...currentPoints],
      size: strokeWidth.value * 4,
    });
  }

  currentPoints = [];
  redraw();
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
    try {
      imageSrc.value = await readImageBase64(path);
    } catch (e) {
      console.error("Failed to load image:", e);
    }
  }
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

.size-label {
  color: #aaa;
  font-size: 12px;
  min-width: 32px;
}
</style>
