/**
 * Typed IPC command wrappers for the Tauri backend.
 *
 * This file provides type-safe wrappers around `invoke` to catch
 * command name typos and argument mismatches at compile time.
 */

import { invoke } from "@tauri-apps/api/core";

// ─────────────────────────────────────────────────────────────
// Capture
// ─────────────────────────────────────────────────────────────

export interface RgbColor {
  r: number;
  g: number;
  b: number;
}

export async function captureScreenRegion(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<string> {
  return invoke("capture_screen_region", { x, y, width, height });
}

export async function createPinFromRegion(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<string> {
  return invoke("create_pin_from_region", { x, y, width, height });
}

export async function saveRegionDialog(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<void> {
  return invoke("save_region_dialog", { x, y, width, height });
}

export async function copyRegionToClipboard(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<void> {
  return invoke("copy_region_to_clipboard", { x, y, width, height });
}

export async function readImageBase64(path: string): Promise<string> {
  return invoke("read_image_base64", { path });
}

export async function getPixelColor(x: number, y: number): Promise<RgbColor> {
  return invoke("get_pixel_color", { x, y });
}

export async function getMagnifierArea(
  x: number,
  y: number,
  size: number
): Promise<string> {
  return invoke("get_magnifier_area", { x, y, size });
}

export async function ocrRegion(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<string> {
  return invoke("ocr_region", { x, y, width, height });
}

export async function startGifRecord(
  x: number,
  y: number,
  width: number,
  height: number,
  fps: number,
  scale: number,
  quality: number
): Promise<void> {
  return invoke("start_gif_record", { x, y, width, height, fps, scale, quality });
}

export async function stopGifRecord(): Promise<string> {
  return invoke("stop_gif_record");
}

// ─────────────────────────────────────────────────────────────
// Window
// ─────────────────────────────────────────────────────────────

export async function createEditorWindow(imagePath: string): Promise<void> {
  return invoke("create_editor_window", { imagePath });
}

export async function closeCaptureWindows(): Promise<void> {
  return invoke("close_capture_windows");
}

export async function closePinWindow(label: string): Promise<void> {
  return invoke("close_pin_window", { label });
}

// ─────────────────────────────────────────────────────────────
// Clipboard
// ─────────────────────────────────────────────────────────────

export async function copyTextToClipboard(text: string): Promise<void> {
  return invoke("copy_text_to_clipboard", { text });
}

// ─────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────

export async function getDoublePressEnabled(): Promise<boolean> {
  return invoke("get_double_press_enabled");
}

export async function setDoublePressEnabled(enabled: boolean): Promise<void> {
  return invoke("set_double_press_enabled", { enabled });
}

export async function getScreenshotHotkey(): Promise<string> {
  return invoke("get_screenshot_hotkey");
}

export async function setScreenshotHotkey(hotkey: string): Promise<void> {
  return invoke("set_screenshot_hotkey", { hotkey });
}

export async function getCopyHotkey(): Promise<string> {
  return invoke("get_copy_hotkey");
}

export async function getGifFps(): Promise<number> {
  return invoke("get_gif_fps");
}

export async function setGifFps(fps: number): Promise<void> {
  return invoke("set_gif_fps", { fps });
}

export async function getGifQuality(): Promise<number> {
  return invoke("get_gif_quality");
}

export async function setGifQuality(quality: number): Promise<void> {
  return invoke("set_gif_quality", { quality });
}

export async function setCopyHotkey(hotkey: string): Promise<void> {
  return invoke("set_copy_hotkey", { hotkey });
}

export async function getSaveHotkey(): Promise<string> {
  return invoke("get_save_hotkey");
}

export async function setSaveHotkey(hotkey: string): Promise<void> {
  return invoke("set_save_hotkey", { hotkey });
}

export async function getPinHotkey(): Promise<string> {
  return invoke("get_pin_hotkey");
}

export async function setPinHotkey(hotkey: string): Promise<void> {
  return invoke("set_pin_hotkey", { hotkey });
}

export async function getOcrHotkey(): Promise<string> {
  return invoke("get_ocr_hotkey");
}

export async function setOcrHotkey(hotkey: string): Promise<void> {
  return invoke("set_ocr_hotkey", { hotkey });
}

