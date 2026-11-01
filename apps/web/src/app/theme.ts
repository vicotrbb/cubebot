export type Theme = 'system' | 'light' | 'dark'
const KEY = 'cubebot:theme'

/** Runs before paint so the first frame carries the chosen theme. */
export const THEME_BOOT = `(function(){try{var t=localStorage.getItem(${JSON.stringify(KEY)});if(t==='light'||t==='dark')document.documentElement.dataset.theme=t;}catch(e){}if('serviceWorker'in navigator)addEventListener('load',function(){navigator.serviceWorker.register('/sw.js').catch(function(){})});})();`

export function applyTheme(theme: Theme) {
  const root = document.documentElement
  if (theme === 'system') delete root.dataset.theme
  else root.dataset.theme = theme
  try {
    if (theme === 'system') localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, theme)
  } catch {
    /* The theme still applies to this page. */
  }
}
