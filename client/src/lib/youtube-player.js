// Listen to an embedded YouTube player through the IFrame Player API. The
// iframe plays on its own; this only adds what the page cannot see otherwise:
// the player's error codes and the video's title.
let api = null

function loadApi() {
  api ??= new Promise((resolve, reject) => {
    if (window.YT?.Player) {
      resolve(window.YT)
      return
    }
    const previous = window.onYouTubeIframeAPIReady
    window.onYouTubeIframeAPIReady = () => {
      previous?.()
      resolve(window.YT)
    }
    const script = document.createElement('script')
    script.src = 'https://www.youtube.com/iframe_api'
    script.onerror = () => {
      api = null
      reject(new Error('iframe_api failed to load'))
    }
    document.head.append(script)
  })
  return api
}

/**
 * Attach to an `<iframe>` whose src has `enablejsapi=1`. Returns a detach
 * function. If the API script cannot load, the iframe keeps playing unobserved.
 */
export function observePlayer(frame, { onError, onTitle }) {
  let player = null
  let detached = false

  loadApi()
    .then((YT) => {
      if (detached) return
      player = new YT.Player(frame, {
        events: {
          onReady: (event) => onTitle(event.target.getVideoData?.().title ?? ''),
          onStateChange: (event) => onTitle(event.target.getVideoData?.().title ?? ''),
          onError: () => onError(),
        },
      })
    })
    .catch(() => {})

  return () => {
    detached = true
    try {
      player?.destroy?.()
    } catch {
      // The iframe is already gone; nothing is left to release.
    }
  }
}
