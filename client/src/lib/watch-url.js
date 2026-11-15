// The open video lives in the query string (`?watch=<id>`), not in the path:
// the list behind it stays mounted, keeps its scroll position, and the browser
// back button closes the player.
const VIDEO_ID = /^[\w-]{11}$/

export function watchIdFromSearch(search) {
  const id = new URLSearchParams(search).get('watch')
  return id && VIDEO_ID.test(id) ? id : null
}

/** The query string with the watch parameter set to `id`, or removed for null. */
export function searchWithWatch(search, id) {
  const params = new URLSearchParams(search)
  if (id) params.set('watch', id)
  else params.delete('watch')
  const next = params.toString()
  return next ? `?${next}` : ''
}
