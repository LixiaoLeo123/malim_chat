/*
 * Identity i18n: the vendored new-api components call useTranslation(), but this
 * client ships English-only. Returning the key verbatim keeps the components
 * byte-identical to upstream while rendering English text.
 */
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

void i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  resources: { en: { translation: {} } },
  interpolation: { escapeValue: false },
  returnEmptyString: false,
  parseMissingKeyHandler: (key) => key,
});

export default i18n;
