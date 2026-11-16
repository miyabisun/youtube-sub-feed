// Running inside an app that opens videos on YouTube itself (subfeed-browser).
// The app marks every page it loads; this page then hands a tapped video over
// instead of letting the link open a new tab.
import { youtubeUrl } from './youtube-url.js'

/** True when the host app marked this page with `data-video-host="1"`. */
export function hasVideoHost(root = globalThis.document?.documentElement) {
  return root?.dataset?.videoHost === '1'
}

/** A plain tap in the host is the host's to open; new-tab gestures stay with the browser. */
export function handsOver(event, root) {
  return (
    hasVideoHost(root) &&
    event.type === 'click' &&
    event.button === 0 &&
    !(event.ctrlKey || event.metaKey || event.shiftKey || event.altKey)
  )
}

/** What the host needs to open a video: its id and its address on YouTube. */
export function openVideoMessage(video) {
  return { source: 'youtube-sub-feed', type: 'open-video', id: video.id, url: youtubeUrl(video) }
}

/** Click handler for a video link: in the host, cancel the link and hand the video over. */
export function handOver(event, video) {
  if (!handsOver(event)) return
  event.preventDefault()
  window.postMessage(openVideoMessage(video), window.location.origin)
}
