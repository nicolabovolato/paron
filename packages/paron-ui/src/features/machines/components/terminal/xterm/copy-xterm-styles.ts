/**
 * xterm.js styles itself with `<style>` elements, and a release build's `style-src 'self'` refuses
 * them (xtermjs/xterm.js#4445). Their CSS, copied into constructed stylesheets, is applied through
 * the CSSOM, which the policy allows; so the DOM renderer draws without WebGL, and nothing in the
 * policy changes. The refused elements still log a violation each; the copies are what apply.
 */
export function copyXtermStyles(element: HTMLElement) {
  const sheets = new Map<HTMLStyleElement, CSSStyleSheet>();

  const adoptCopies = () => {
    const copies = new Set(sheets.values());
    document.adoptedStyleSheets = [
      ...document.adoptedStyleSheets.filter((sheet) => !copies.has(sheet)),
      ...copies,
    ];
  };

  const copyStyles = () => {
    let added = false;
    for (const style of element.querySelectorAll("style")) {
      let sheet = sheets.get(style);
      if (!sheet) {
        sheet = new CSSStyleSheet();
        sheets.set(style, sheet);
        added = true;
      }
      sheet.replaceSync(style.textContent);
    }
    for (const [style, sheet] of sheets) {
      if (style.isConnected) continue;
      sheets.delete(style);
      document.adoptedStyleSheets = document.adoptedStyleSheets.filter(
        (adopted) => adopted !== sheet,
      );
    }
    if (added) adoptCopies();
  };

  const observer = new MutationObserver(copyStyles);
  observer.observe(element, {
    subtree: true,
    childList: true,
    characterData: true,
  });
  copyStyles();

  return () => {
    observer.disconnect();
    const copies = new Set(sheets.values());
    document.adoptedStyleSheets = document.adoptedStyleSheets.filter(
      (sheet) => !copies.has(sheet),
    );
  };
}
