export type Locale = "zh-CN" | "en-US";

export const DEFAULT_LOCALE: Locale = "zh-CN";
export const SUPPORTED_LOCALES: readonly Locale[] = ["zh-CN", "en-US"];

export type MessageDict = { [key: string]: string | MessageDict };
export type MessageParams = Record<string, string | number>;

export function isLocale(value: unknown): value is Locale {
  return value === "zh-CN" || value === "en-US";
}

export function normalizeLocale(value: unknown): Locale {
  return isLocale(value) ? value : DEFAULT_LOCALE;
}

export function interpolate(template: string, params?: MessageParams): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

export function lookup(dict: MessageDict, key: string): string | undefined {
  const parts = key.split(".");
  let node: string | MessageDict | undefined = dict;
  for (const part of parts) {
    if (typeof node !== "object" || node === null) return undefined;
    node = node[part];
    if (node === undefined) return undefined;
  }
  return typeof node === "string" ? node : undefined;
}

export function flattenKeys(dict: MessageDict, prefix = ""): string[] {
  const keys: string[] = [];
  for (const [name, value] of Object.entries(dict)) {
    const key = prefix ? `${prefix}.${name}` : name;
    if (typeof value === "string") keys.push(key);
    else keys.push(...flattenKeys(value, key));
  }
  return keys.sort();
}

export function diffKeys(a: MessageDict, b: MessageDict): { missing: string[]; extra: string[] } {
  const keysA = new Set(flattenKeys(a));
  const keysB = new Set(flattenKeys(b));
  return {
    missing: [...keysA].filter((key) => !keysB.has(key)).sort(),
    extra: [...keysB].filter((key) => !keysA.has(key)).sort(),
  };
}
