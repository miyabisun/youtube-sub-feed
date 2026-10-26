/**
 * Mirror the rendered page background into <meta name="theme-color"> and keep it
 * in step with the OS light/dark switch. Installed web apps on Android Chrome take
 * the navigation-button contrast from it, so without it the Washi theme gets white
 * buttons on a white bar. The --c-bg token in global.sass stays the source.
 */
export function followThemeColor() {
  const meta = document.createElement('meta')
  meta.name = 'theme-color'
  document.head.append(meta)

  function sync() {
    meta.content = getComputedStyle(document.body).backgroundColor
  }

  sync()
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', sync)
}
