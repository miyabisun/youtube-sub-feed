// Running inside an app that plays videos on YouTube itself (subfeed-browser).
// The app marks every page it loads; this page then hands a video over
// instead of playing it in the embedded watch view.
import { youtubeUrl } from './youtube-url.js'

/** True when the host app marked this page with `data-video-host="1"`. */
export function hasVideoHost(root = globalThis.document?.documentElement) {
  return root?.dataset?.videoHost === '1'
}

/** What the host needs to open a video: its id and its address on YouTube. */
export function openVideoMessage(video) {
  return { source: 'youtube-sub-feed', type: 'open-video', id: video.id, url: youtubeUrl(video) }
}
