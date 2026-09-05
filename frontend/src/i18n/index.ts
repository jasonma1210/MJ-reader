import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhCN from "./locales/zh-CN.json";
import en from "./locales/en.json";

const SUPPORTED = ["zh-CN", "en"] as const;
type Lang = (typeof SUPPORTED)[number];

function detectInitialLang(): Lang {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem("mjnexus-lang");
    if (saved && (SUPPORTED as readonly string[]).includes(saved)) {
      return saved as Lang;
    }
  }
  if (typeof navigator !== "undefined" && navigator.language.startsWith("zh")) {
    return "zh-CN";
  }
  return "en";
}

const initialLang = detectInitialLang();

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    en: { translation: en },
  },
  lng: initialLang,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export function setLanguage(lang: Lang): void {
  void i18n.changeLanguage(lang);
  if (typeof localStorage !== "undefined") {
    localStorage.setItem("mjnexus-lang", lang);
  }
}

export default i18n;
