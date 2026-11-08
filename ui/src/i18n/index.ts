import en from "./en.json";

export type MessageKey = keyof typeof en;
const messages: Record<string, string> = en;

/** Look up a user-facing string and fill `{{var}}` placeholders. */
export function t(key: MessageKey, vars: Record<string, string | number> = {}): string {
  const template = messages[key] ?? key;
  return template.replace(/\{\{\s*(\w+)\s*\}\}/g, (match, name: string) =>
    name in vars ? String(vars[name]) : match,
  );
}

/** For keys built at runtime (e.g. from enum values); falls back to the key itself. */
export function tDynamic(key: string, vars: Record<string, string | number> = {}): string {
  return key in messages ? t(key as MessageKey, vars) : key;
}
