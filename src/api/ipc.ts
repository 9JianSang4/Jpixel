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

export interface StrokePoint {
  x: number;
  y: number;
}

export interface PenStroke {
  type: "pen";
  points: StrokePoint[];
  color: string;
  width: number;
}

export interface EraserStroke {
  type: "eraser";
  points: StrokePoint[];
  size: number;
}

export type DrawStroke = PenStroke | EraserStroke;

export async function createPinFromRegion(
  x: number,
  y: number,
  width: number,
  height: number,
  strokes?: DrawStroke[]
): Promise<string> {
  return invoke("create_pin_from_region", { x, y, width, height, strokes: strokes ? JSON.stringify(strokes) : null });
}

export async function saveRegionDialog(
  x: number,
  y: number,
  width: number,
  height: number,
  strokes?: DrawStroke[]
): Promise<void> {
  return invoke("save_region_dialog", { x, y, width, height, strokes: strokes ? JSON.stringify(strokes) : null });
}

export async function copyRegionToClipboard(
  x: number,
  y: number,
  width: number,
  height: number,
  strokes?: DrawStroke[]
): Promise<void> {
  return invoke("copy_region_to_clipboard", { x, y, width, height, strokes: strokes ? JSON.stringify(strokes) : null });
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

// ─────────────────────────────────────────────────────────────
// Window
// ─────────────────────────────────────────────────────────────

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
  return invoke("set_double_press_enabled", { value: enabled });
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

export async function getAutoLaunch(): Promise<boolean> {
  return invoke("get_auto_launch");
}

export async function setAutoLaunch(enabled: boolean): Promise<void> {
  return invoke("set_auto_launch", { value: enabled });
}

export async function getDefaultAction(): Promise<string> {
  return invoke("get_default_action");
}

export async function setDefaultAction(action: string): Promise<void> {
  return invoke("set_default_action", { value: action });
}

