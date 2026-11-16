import {
  createContext,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import zhCN from "../locales/zh-CN.json";
import enUS from "../locales/en-US.json";
import {
  interpolate,
  lookup,
  normalizeLocale,
  type Locale,
  type MessageDict,
  type MessageParams,
} from "./i18n-core";

const dictionaries: Record<Locale, MessageDict> = {
  "zh-CN": zhCN as MessageDict,
  "en-US": enUS as MessageDict,
};

interface I18nValue {
  locale: Locale;
  t: (key: string, params?: MessageParams) => string;
  setLocale: (locale: Locale) => void;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({
  locale: rawLocale,
  onLocaleChange,
  children,
}: {
  locale: Locale | string;
  onLocaleChange: (locale: Locale) => void;
  children: ReactNode;
}) {
  const locale = normalizeLocale(rawLocale);
  const value = useMemo<I18nValue>(() => {
    const dict = dictionaries[locale];
    return {
      locale,
      t(key, params) {
        const template = lookup(dict, key);
        if (template !== undefined) return interpolate(template, params);
        const fallback = lookup(dictionaries["zh-CN"], key);
        if (import.meta.env.DEV) {
          console.warn(`[i18n] missing key: ${key} (${locale})`);
        }
        return fallback !== undefined ? interpolate(fallback, params) : `⟦${key}⟧`;
      },
      setLocale: onLocaleChange,
    };
  }, [locale, onLocaleChange]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n must be used inside I18nProvider");
  return value;
}

export function getDictionary(locale: Locale): MessageDict {
  return dictionaries[locale];
}
