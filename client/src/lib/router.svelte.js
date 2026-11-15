import { watchIdFromSearch, searchWithWatch } from './watch-url.js'

export function getBasePath() {
  return (window.__BASE_PATH__ || '').replace(/\/+$/, '')
}

export function link(path) {
  return `${getBasePath()}${path}`
}

let _routeIndex = $state(0)
let _params = $state({})
// The video open in the watch view: its id comes from the URL, the rest of
// the card (title, channel) only when it was opened from a list.
let _watchId = $state(null)
let _watchVideo = $state(null)

export const routes = [
  { pattern: /^\/$/, params: [] },
  { pattern: /^\/group\/([^/]+)$/, params: ['id'] },
  { pattern: /^\/channels$/, params: [] },
  { pattern: /^\/channel\/([^/]+)$/, params: ['id'] },
  { pattern: /^\/login$/, params: [] },
  { pattern: /^\/settings$/, params: [] },
  { pattern: /^\/history$/, params: [] },
]

export function matchRoute(path) {
  for (let i = 0; i < routes.length; i++) {
    const match = path.match(routes[i].pattern)
    if (match) {
      const params = {}
      routes[i].params.forEach((key, j) => {
        params[key] = decodeURIComponent(match[j + 1])
      })
      return { index: i, params }
    }
  }
  return { index: 0, params: {} }
}

function getPathFromURL() {
  const base = getBasePath()
  let path = window.location.pathname
  if (base && path.startsWith(base)) {
    path = path.slice(base.length) || '/'
  }
  return path
}

function syncRoute() {
  const result = matchRoute(getPathFromURL())
  _routeIndex = result.index
  _params = result.params
  _watchId = watchIdFromSearch(window.location.search)
}

export function navigate(path) {
  history.pushState({}, '', getBasePath() + path)
  syncRoute()
}

/** Open a video over the current page; the browser back button closes it. */
export function openWatch(video) {
  _watchVideo = video
  const { pathname, search } = window.location
  history.pushState({ watch: video.id }, '', pathname + searchWithWatch(search, video.id))
  syncRoute()
}

export function closeWatch() {
  if (!_watchId) return
  // Opened here: step back to the entry it was opened from. Reached by a
  // reload or a shared link: there is no such entry, so drop the parameter.
  if (history.state?.watch) {
    history.back()
    return
  }
  const { pathname, search } = window.location
  history.replaceState({}, '', pathname + searchWithWatch(search, null))
  syncRoute()
}

window.addEventListener('popstate', syncRoute)

// Initialize on load
syncRoute()

export const router = {
  get index() {
    return _routeIndex
  },
  get params() {
    return _params
  },
  get watch() {
    if (!_watchId) return null
    return { id: _watchId, video: _watchVideo?.id === _watchId ? _watchVideo : null }
  },
}
