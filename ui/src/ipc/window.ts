// Window-level helpers (current window label, dragging, native context menus).
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "./events";

export function currentWindowLabel(): string {
  if (!isTauri()) return "main";
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

export function startDragging(): Promise<void> {
  return getCurrentWindow().startDragging();
}

export interface ContextMenuItem {
  text: string;
  action: () => void;
}

/** Show a native context menu at the cursor. `null` entries become separators. */
export async function showContextMenu(items: (ContextMenuItem | null)[]): Promise<void> {
  const menu = await Menu.new({
    items: items.map((i) => (i === null ? { item: "Separator" as const } : { text: i.text, action: i.action })),
  });
  await menu.popup();
}
