/**
 * Typed IPC command wrappers for the Tauri backend.
 *
 * This file provides type-safe wrappers around `invoke` to catch
 * command name typos and argument mismatches at compile time.
 */

import { invoke } from "@tauri-apps/api/core";

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

export type PinLimit = "four" | "eight" | "sixteen" | "thirty_two";

export async function getPinLimit(): Promise<PinLimit> {
  return invoke("get_pin_limit");
}

export async function setPinLimit(limit: PinLimit): Promise<void> {
  return invoke("set_pin_limit", { value: limit });
}

export async function getAutoLaunch(): Promise<boolean> {
  return invoke("get_auto_launch");
}

export async function setAutoLaunch(enabled: boolean): Promise<void> {
  return invoke("set_auto_launch", { value: enabled });
}

export type SelectionMode = "reselect" | "resize";

export type DoublePressSpeed = "slow" | "normal" | "fast";

export async function getDoublePressSpeed(): Promise<DoublePressSpeed> {
  return invoke("get_double_press_speed");
}

export async function setDoublePressSpeed(speed: DoublePressSpeed): Promise<void> {
  return invoke("set_double_press_speed", { value: speed });
}

export async function getCrosshairGuides(): Promise<boolean> {
  return invoke("get_crosshair_guides");
}

export async function setCrosshairGuides(enabled: boolean): Promise<void> {
  return invoke("set_crosshair_guides", { value: enabled });
}

export async function getMagnifierEnabled(): Promise<boolean> {
  return invoke("get_magnifier_enabled");
}

export async function setMagnifierEnabled(enabled: boolean): Promise<void> {
  return invoke("set_magnifier_enabled", { value: enabled });
}

export async function getSelectionMode(): Promise<SelectionMode> {
  return invoke("get_selection_mode");
}

export async function setSelectionMode(mode: SelectionMode): Promise<void> {
  return invoke("set_selection_mode", { value: mode });
}

