/**
 * Format a KeyboardEvent into a human-readable hotkey string.
 *
 * Example output: "Ctrl+Shift+A", "F1", "Alt+Enter"
 */
export function formatHotkey(e: KeyboardEvent): string {
  const modifiers: string[] = [];
  if (e.ctrlKey) modifiers.push("Ctrl");
  if (e.altKey) modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey) modifiers.push("Super");

  let key = e.key;
  if (key === "Control" || key === "Alt" || key === "Shift" || key === "Meta") {
    return "";
  }
  if (key === " ") key = "Space";
  if (key.length === 1) key = key.toUpperCase();

  const parts = [...modifiers, key];
  return parts.join("+");
}
