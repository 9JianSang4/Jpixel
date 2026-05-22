<template>
  <div class="capture-layer" :style="{ cursor: cursorStyle }" @mousedown="onMouseDown" @mousemove="onMouseMove" @mouseup="onMouseUp" @dblclick="onConfirm" @contextmenu.prevent tabindex="0" ref="layerRef">
    <!-- Crosshair cursor lines (segmented to skip 120x120 area around cursor) -->
    <template v-if="!isDragging && !hasSelection">
      <!-- Horizontal left segment -->
      <div class="crosshair-h" :style="{ transform: `translateY(${cursorY}px)`, width: `${Math.max(0, cursorX - 60)}px` }" />
      <!-- Horizontal right segment -->
      <div class="crosshair-h" :style="{ transform: `translateY(${cursorY}px)`, left: `${cursorX + 60}px`, width: `calc(100vw - ${cursorX + 60}px)` }" />
      <!-- Vertical top segment -->
      <div class="crosshair-v" :style="{ transform: `translateX(${cursorX}px)`, height: `${Math.max(0, cursorY - 60)}px` }" />
      <!-- Vertical bottom segment -->
      <div class="crosshair-v" :style="{ transform: `translateX(${cursorX}px)`, top: `${cursorY + 60}px`, height: `calc(100vh - ${cursorY + 60}px)` }" />
    </template>

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
      <button class="tool-btn" @click.stop="onConfirm" title="复制到剪贴板 (Enter)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
      </button>
      <div class="divider" />
      <button
        class="tool-btn"
        :class="{ active: annotationTool === 'pen' }"
        @click.stop="selectAnnotationTool('pen')"
        title="画笔"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 19l7-7 3 3-7 7-3-3z"/><path d="M18 13l-1.5-7.5L2 2l3.5 14.5L13 18l5-5z"/>
        </svg>
      </button>
      <button
        class="tool-btn"
        :class="{ active: annotationTool === 'eraser' }"
        @click.stop="selectAnnotationTool('eraser')"
        title="擦除"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M20 20H7L3 16c-.8-.8-.8-2 0-2.8l9.2-9.2c.8-.8 2-.8 2.8 0L20 8.8c.8.8.8 2 0 2.8L11 20"/>
        </svg>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click.stop="onOcr" title="文字识别 (Ctrl+R)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 7V4h16v3"/><path d="M9 20h6"/><path d="M12 4v16"/></svg>
      </button>
      <div class="divider" />
      <button class="tool-btn" @click.stop="onCancel" title="取消 (Esc)">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
      </button>
    </div>

    <!-- Drawing canvas overlay (always shown when selected, pointer-events via class) -->
    <canvas
      v-show="hasSelection"
      ref="drawCanvasRef"
      class="draw-canvas"
      :class="{ active: !!annotationTool }"
      :style="selectionStyle"
      @mousedown.stop="onDrawStart"
      @mousemove.stop="onDrawMove"
      @mouseup.stop="onDrawEnd"
      @mouseleave.stop="onDrawEnd"
    />

    <!-- Magnifier + Color picker -->
    <div v-if="showMagnifier" class="magnifier" :style="magnifierStyle">
      <div class="magnifier-img-wrapper">
        <img v-if="magnifierSrc" :src="magnifierSrc" class="magnifier-img" />
        <div class="magnifier-crosshair" />
      </div>
      <div class="color-info">{{ hexColor }}</div>
      <div class="color-info rgb">{{ rgbColor }}</div>
      <div class="color-hint">按 C 复制</div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import {
  copyRegionToClipboard,
  saveRegionDialog,
  createPinFromRegion,
  closeCaptureWindows,
  copyTextToClipboard,
  getPixelColor,
  getMagnifierArea,
  ocrRegion,
  getCopyHotkey,
  getSaveHotkey,
  getPinHotkey,
  getOcrHotkey,
  DrawStroke,
  StrokePoint,
} from "../api/ipc";

const layerRef = ref<HTMLDivElement | null>(null);
const cursorX = ref(0);
const cursorY = ref(0);

const startX = ref(0);
const startY = ref(0);
const endX = ref(0);
const endY = ref(0);
const isDragging = ref(false);
const hasSelection = ref(false);
type DragMode = 'create' | 'move' | 'resize-nw' | 'resize-n' | 'resize-ne' | 'resize-w' | 'resize-e' | 'resize-sw' | 'resize-s' | 'resize-se';
const dragMode = ref<DragMode>('create');
const moveOffsetX = ref(0);
const moveOffsetY = ref(0);
const cursorStyle = ref('crosshair');

const pixelColor = ref({ r: 0, g: 0, b: 0 });
const magnifierSrc = ref("");
const showMagnifier = ref(true);

// Annotation (pen / eraser) state
const drawCanvasRef = ref<HTMLCanvasElement | null>(null);
const annotationTool = ref<"pen" | "eraser" | null>(null);
const strokes = ref<DrawStroke[]>([]);
let isDrawingStroke = false;
let currentStroke: StrokePoint[] = [];

const copyHotkey = ref("Ctrl+C");
const saveHotkey = ref("Ctrl+S");
const pinHotkey = ref("Ctrl+T");
const ocrHotkey = ref("Ctrl+R");
const route = useRoute();
const winOffset = ref({
  x: parseInt((route.query.ox as string) || "0"),
  y: parseInt((route.query.oy as string) || "0"),
});
const dpr = ref(parseFloat((route.query.scale as string) || "1"));

// Convert CSS-pixel coordinate + physical origin to absolute physical pixels.
// The screenshot backend uses physical pixel coordinates, but mouse events
// in the WebView report CSS pixels (== logical pixels).
function toPhysical(cssCoord: number, physicalOrigin: number): number {
  return physicalOrigin + Math.round(cssCoord * dpr.value);
}

const HANDLE = 8;
const THROTTLE = 50;
const MIN_SELECTION_SIZE = 2;

let pendingTimeout: number | null = null;
let isUnmounted = false;

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

const hexColor = computed(() => {
  const { r, g, b } = pixelColor.value;
  const toHex = (n: number) => n.toString(16).padStart(2, "0").toUpperCase();
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
});

const rgbColor = computed(() => {
  const { r, g, b } = pixelColor.value;
  return `RGB(${r}, ${g}, ${b})`;
});

const magnifierStyle = computed(() => {
  const panelW = 120;
  const panelH = 150;
  const offset = 20;

  let left = cursorX.value + offset;
  let top = cursorY.value + offset;

  if (left + panelW > window.innerWidth) {
    left = cursorX.value - panelW - offset;
  }
  if (top + panelH > window.innerHeight) {
    top = cursorY.value - panelH - offset;
  }

  return {
    left: `${left}px`,
    top: `${top}px`,
  };
});

type Zone = 'nw' | 'n' | 'ne' | 'w' | 'move' | 'e' | 'sw' | 's' | 'se' | 'outside';

function getCursorZone(x: number, y: number): Zone {
  if (!hasSelection.value) return 'outside';
  const left = selLeft.value;
  const top = selTop.value;
  const right = selRight.value;
  const bottom = selBottom.value;

  if (x < left - HANDLE || x > right + HANDLE || y < top - HANDLE || y > bottom + HANDLE) {
    return 'outside';
  }

  const onLeft = Math.abs(x - left) <= HANDLE;
  const onRight = Math.abs(x - right) <= HANDLE;
  const onTop = Math.abs(y - top) <= HANDLE;
  const onBottom = Math.abs(y - bottom) <= HANDLE;

  if (onTop && onLeft) return 'nw';
  if (onTop && onRight) return 'ne';
  if (onBottom && onLeft) return 'sw';
  if (onBottom && onRight) return 'se';
  if (onTop) return 'n';
  if (onBottom) return 's';
  if (onLeft) return 'w';
  if (onRight) return 'e';

  return 'move';
}

function zoneToCursor(zone: Zone): string {
  switch (zone) {
    case 'nw':
    case 'se': return 'nwse-resize';
    case 'ne':
    case 'sw': return 'nesw-resize';
    case 'n':
    case 's': return 'ns-resize';
    case 'w':
    case 'e': return 'ew-resize';
    case 'move': return 'move';
    default: return 'crosshair';
  }
}

let pendingMagnifier = false;

async function updateMagnifierAndColor(x: number, y: number) {
  if (pendingMagnifier) return;
  pendingMagnifier = true;
  try {
    const color = await getPixelColor(x, y);
    if (isUnmounted) return;
    pixelColor.value = { r: color.r, g: color.g, b: color.b };

    const src = await getMagnifierArea(x, y, 15);
    if (isUnmounted) return;
    magnifierSrc.value = src;
  } catch (e) {
    if (!isUnmounted) {
      console.error("Magnifier error:", e);
    }
  } finally {
    pendingMagnifier = false;
  }
}

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;

  if (hasSelection.value) {
    const zone = getCursorZone(e.clientX, e.clientY);
    if (zone !== 'outside') {
      if (zone === 'move') {
        dragMode.value = 'move';
        moveOffsetX.value = e.clientX - selLeft.value;
        moveOffsetY.value = e.clientY - selTop.value;
      } else {
        dragMode.value = `resize-${zone}`;
      }
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

  if (!isDragging.value) {
    if (hasSelection.value) {
      const zone = getCursorZone(e.clientX, e.clientY);
      cursorStyle.value = zoneToCursor(zone);
    } else {
      cursorStyle.value = 'crosshair';
    }
  }

  if (isDragging.value) {
    if (dragMode.value === 'move') {
      const newLeft = e.clientX - moveOffsetX.value;
      const newTop = e.clientY - moveOffsetY.value;
      const w = selWidth.value;
      const h = selHeight.value;
      startX.value = newLeft;
      startY.value = newTop;
      endX.value = newLeft + w;
      endY.value = newTop + h;
    } else if (dragMode.value.startsWith('resize-')) {
      const zone = dragMode.value.replace('resize-', '');
      if (zone.includes('n')) startY.value = e.clientY;
      if (zone.includes('s')) endY.value = e.clientY;
      if (zone.includes('w')) startX.value = e.clientX;
      if (zone.includes('e')) endX.value = e.clientX;
    } else {
      endX.value = e.clientX;
      endY.value = e.clientY;
    }
  }

  // Throttle magnifier updates
  if (pendingTimeout === null) {
    pendingTimeout = window.setTimeout(() => {
      pendingTimeout = null;
      updateMagnifierAndColor(
        toPhysical(cursorX.value, winOffset.value.x),
        toPhysical(cursorY.value, winOffset.value.y)
      );
    }, THROTTLE);
  }
}

function clampSelection() {
  const min = MIN_SELECTION_SIZE;
  if (selWidth.value < min) {
    if (startX.value <= endX.value) {
      endX.value = startX.value + min;
    } else {
      startX.value = endX.value + min;
    }
  }
  if (selHeight.value < min) {
    if (startY.value <= endY.value) {
      endY.value = startY.value + min;
    } else {
      startY.value = endY.value + min;
    }
  }
}

function onMouseUp() {
  isDragging.value = false;
  clampSelection();
}

async function onConfirm() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) {
    onCancel();
    return;
  }
  // Enter / ✓ = copy to clipboard (with drawings composited)
  await onCopyRegion();
}

function onCancel() {
  closeCaptureWindows();
}

function matchHotkey(e: KeyboardEvent, hotkey: string): boolean {
  const parts = hotkey.split("+").map((p) => p.trim());
  let ctrl = false, shift = false, alt = false, meta = false;
  let key = "";
  for (const part of parts) {
    if (part === "Ctrl") ctrl = true;
    else if (part === "Shift") shift = true;
    else if (part === "Alt") alt = true;
    else if (part === "Super" || part === "Cmd" || part === "Meta" || part === "Win") meta = true;
    else key = part;
  }
  if (key === " ") key = "Space";
  if (key.length === 1) key = key.toUpperCase();
  return (
    e.ctrlKey === ctrl &&
    e.shiftKey === shift &&
    e.altKey === alt &&
    e.metaKey === meta &&
    e.key.toUpperCase() === key.toUpperCase()
  );
}

async function onCopyRegion() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) return;
  try {
    await copyRegionToClipboard(
      toPhysical(selLeft.value, winOffset.value.x),
      toPhysical(selTop.value, winOffset.value.y),
      Math.round(selWidth.value * dpr.value),
      Math.round(selHeight.value * dpr.value),
      strokes.value.length > 0 ? strokes.value : undefined
    );
    closeCaptureWindows();
  } catch (e) {
    console.error("Copy failed:", e);
  }
}

async function onSaveRegion() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) return;
  try {
    await saveRegionDialog(
      toPhysical(selLeft.value, winOffset.value.x),
      toPhysical(selTop.value, winOffset.value.y),
      Math.round(selWidth.value * dpr.value),
      Math.round(selHeight.value * dpr.value),
      strokes.value.length > 0 ? strokes.value : undefined
    );
  } catch (e) {
    console.error("Save failed:", e);
  }
}

async function onPinRegion() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) return;
  try {
    await createPinFromRegion(
      toPhysical(selLeft.value, winOffset.value.x),
      toPhysical(selTop.value, winOffset.value.y),
      Math.round(selWidth.value * dpr.value),
      Math.round(selHeight.value * dpr.value),
      strokes.value.length > 0 ? strokes.value : undefined
    );
  } catch (e) {
    console.error("Pin failed:", e);
  }
}

async function onOcr() {
  if (!hasSelection.value || selWidth.value < 2 || selHeight.value < 2) return;
  try {
    const text = await ocrRegion(
      toPhysical(selLeft.value, winOffset.value.x),
      toPhysical(selTop.value, winOffset.value.y),
      Math.round(selWidth.value * dpr.value),
      Math.round(selHeight.value * dpr.value)
    );
    await copyTextToClipboard(text);
    alert(`OCR 结果已复制到剪贴板:\n${text}`);
    closeCaptureWindows();
  } catch (e) {
    console.error("OCR failed:", e);
    alert("OCR 识别失败，请检查 Tesseract 是否已安装并加入 PATH。");
  }
}

// ── Annotation tools (pen / eraser) ────────────────────────

function selectAnnotationTool(tool: "pen" | "eraser") {
  annotationTool.value = annotationTool.value === tool ? null : tool;
  setTimeout(resizeDrawCanvas, 0);
}

function resizeDrawCanvas() {
  const canvas = drawCanvasRef.value;
  if (!canvas) return;
  const w = Math.round(selWidth.value * dpr.value);
  const h = Math.round(selHeight.value * dpr.value);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  redrawDrawings();
}

function redrawDrawings() {
  const canvas = drawCanvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  for (const stroke of strokes.value) {
    if (stroke.type === "pen") {
      drawPenStroke(ctx, stroke.points, stroke.color, stroke.width);
    } else {
      drawEraserStroke(ctx, stroke.points, stroke.size);
    }
  }
}

function drawPenStroke(
  ctx: CanvasRenderingContext2D,
  points: StrokePoint[],
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
  points: StrokePoint[],
  size: number,
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.globalCompositeOperation = "destination-out";
  ctx.strokeStyle = "rgba(0,0,0,1)";
  ctx.lineWidth = size;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    ctx.lineTo(points[i].x, points[i].y);
  }
  ctx.stroke();
  ctx.restore();
}

function getDrawCoords(e: MouseEvent): StrokePoint {
  return {
    x: (e.clientX - selLeft.value) * dpr.value,
    y: (e.clientY - selTop.value) * dpr.value,
  };
}

function onDrawStart(e: MouseEvent) {
  if (e.button !== 0 || !annotationTool.value) return;
  isDrawingStroke = true;
  currentStroke = [getDrawCoords(e)];
}

function onDrawMove(e: MouseEvent) {
  if (!isDrawingStroke || !annotationTool.value) return;
  currentStroke.push(getDrawCoords(e));
  redrawDrawings();

  // Draw in-progress stroke preview
  const canvas = drawCanvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  if (annotationTool.value === "pen") {
    drawPenStroke(ctx, currentStroke, "#FF0000", 3);
  } else {
    drawEraserStroke(ctx, currentStroke, 12);
  }
}

function onDrawEnd() {
  if (!isDrawingStroke || !annotationTool.value) return;
  isDrawingStroke = false;

  if (annotationTool.value === "pen") {
    strokes.value.push({
      type: "pen",
      points: [...currentStroke],
      color: "#FF0000",
      width: 3,
    });
  } else {
    strokes.value.push({
      type: "eraser",
      points: [...currentStroke],
      size: 12,
    });
  }
  currentStroke = [];
  redrawDrawings();
}

function onKeyDown(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    onConfirm();
    return;
  }
  if (e.key === "Escape") {
    e.preventDefault();
    onCancel();
    return;
  }
  if (e.key.toLowerCase() === "c" && !e.ctrlKey && !e.altKey && !e.metaKey) {
    e.preventDefault();
    copyTextToClipboard(hexColor.value);
    return;
  }
  // Check by configured hotkey strings.
  if (matchHotkey(e, copyHotkey.value)) {
    e.preventDefault();
    onCopyRegion();
    return;
  }
  if (matchHotkey(e, saveHotkey.value)) {
    e.preventDefault();
    onSaveRegion();
    return;
  }
  if (matchHotkey(e, pinHotkey.value)) {
    e.preventDefault();
    onPinRegion();
    return;
  }
  if (matchHotkey(e, ocrHotkey.value)) {
    e.preventDefault();
    onOcr();
    return;
  }
  // Fallback: match by physical key code (works regardless of hotkey string).
  // This ensures Ctrl+C/S/T/R always work even if the config value is unusual.
  if (!e.repeat && e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
    switch (e.code) {
      case "KeyC":
        e.preventDefault();
        onCopyRegion();
        return;
      case "KeyS":
        e.preventDefault();
        onSaveRegion();
        return;
      case "KeyT":
        e.preventDefault();
        onPinRegion();
        return;
      case "KeyR":
        e.preventDefault();
        onOcr();
        return;
    }
  }
}

// Resize canvas when selection changes
watch(hasSelection, (val) => {
  if (val) setTimeout(resizeDrawCanvas, 0);
});

onMounted(async () => {
  layerRef.value?.focus();
  window.addEventListener("keydown", onKeyDown);
  try {
    copyHotkey.value = await getCopyHotkey();
    saveHotkey.value = await getSaveHotkey();
    pinHotkey.value = await getPinHotkey();
    ocrHotkey.value = await getOcrHotkey();
  } catch (e) {
    console.error("Failed to load hotkeys:", e);
  }
  updateMagnifierAndColor(
    toPhysical(cursorX.value, winOffset.value.x),
    toPhysical(cursorY.value, winOffset.value.y)
  );
});

onUnmounted(() => {
  isUnmounted = true;
  window.removeEventListener("keydown", onKeyDown);
  if (pendingTimeout !== null) {
    clearTimeout(pendingTimeout);
    pendingTimeout = null;
  }
});
</script>

<style scoped>
.capture-layer {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
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
  will-change: transform;
}

.crosshair-v {
  position: fixed;
  top: 0;
  width: 1px;
  height: 100vh;
  background: rgba(255, 255, 255, 0.4);
  pointer-events: none;
  z-index: 10;
  will-change: transform;
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

.tool-btn.recording {
  background: rgba(255, 50, 50, 0.6);
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.divider {
  width: 1px;
  height: 18px;
  background: rgba(255, 255, 255, 0.15);
}

/* Drawing canvas overlay */
.draw-canvas {
  position: fixed;
  z-index: 15;
  pointer-events: none;
  image-rendering: auto;
}
.draw-canvas.active {
  pointer-events: auto;
  cursor: crosshair;
}

/* Magnifier + Color picker */
.magnifier {
  position: fixed;
  padding: 8px;
  border-radius: 10px;
  background: rgba(30, 30, 40, 0.85);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  border: 1px solid rgba(255, 255, 255, 0.1);
  z-index: 30;
  pointer-events: none;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.magnifier-img-wrapper {
  position: relative;
  width: 100px;
  height: 100px;
}

.magnifier-img {
  width: 100%;
  height: 100%;
  image-rendering: pixelated;
  display: block;
  border-radius: 4px;
}

.magnifier-crosshair {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 100%;
  height: 100%;
  transform: translate(-50%, -50%);
  pointer-events: none;
}

.magnifier-crosshair::before,
.magnifier-crosshair::after {
  content: "";
  position: absolute;
  background: rgba(255, 0, 0, 0.7);
}

.magnifier-crosshair::before {
  width: 1px;
  height: 100%;
  left: 50%;
  top: 0;
}

.magnifier-crosshair::after {
  width: 100%;
  height: 1px;
  top: 50%;
  left: 0;
}

.color-info {
  font-size: 11px;
  font-family: monospace;
  color: #fff;
  text-align: center;
  line-height: 1.3;
}

.color-info.rgb {
  color: rgba(255, 255, 255, 0.7);
}

.color-hint {
  font-size: 10px;
  color: rgba(255, 255, 255, 0.4);
  text-align: center;
}
</style>
