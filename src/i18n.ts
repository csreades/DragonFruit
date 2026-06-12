import { i18n } from "@lingui/core";

// Bootstrap: activate English immediately so components render without waiting.
// For the source locale an empty catalog is correct — strings pass through as-is.
i18n.load("en", {});
i18n.activate("en");

// Dynamically load and activate a compiled locale catalog.
// Call this to switch language at runtime.
export async function loadLocale(locale: string): Promise<void> {
  const { messages } = await import(`./locales/${locale}/messages`);
  i18n.load(locale, messages);
  i18n.activate(locale);
}

export { i18n };
