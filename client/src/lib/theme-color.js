/**
 * Mirror the rendered page background into the page's <meta name="theme-color"> and
 * keep it in step with the OS light/dark switch. Chrome colours a tab's system bars
 * from it. An installed app uses the standalone meta in index.html instead, since its
 * buttons sit over the fixed manifest backdrop. The --c-bg token in global.sass stays
 * the source.
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
