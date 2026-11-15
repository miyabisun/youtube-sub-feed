<script module>
  // Kept for the visit, per video: reopening a video shows the like it was left with.
  const known = new Map()
</script>

<script>
  import { untrack } from 'svelte'
  import { tokens } from '$lib/viewer-token.js'
  import { sendAsViewer } from '$lib/youtube-write.js'
  import {
    isLiked,
    rateVideoRequest,
    ratingErrorMessage,
    ratingRequest,
    ratingsDisabled,
  } from '$lib/rating.js'
  import Icon from '$lib/components/Icon.svelte'

  // One instance per video: the parent re-creates this component when the video changes.
  // `notice` is what the parent shows near the button: { text, failed }.
  let { videoId, notice = $bindable(null) } = $props()

  let liked = $state(untrack(() => known.get(videoId) ?? false))
  let sending = $state(false)
  let unavailable = $state(false)
  let touched = false

  // Reading the viewer's rating needs their token, and showing a state is not
  // worth a Google popup: read it only when this visit is already authorized.
  if (tokens.peek()) {
    sendAsViewer(ratingRequest(videoId), { tokens })
      .then((body) => {
        if (!touched) remember(isLiked(body, videoId))
      })
      .catch(() => {})
  }

  function remember(value) {
    liked = value
    known.set(videoId, value)
  }

  async function toggle() {
    if (sending) return
    touched = true
    sending = true
    notice = null
    try {
      await sendAsViewer(rateVideoRequest(videoId, !liked), { tokens })
      remember(!liked)
    } catch (error) {
      if (ratingsDisabled(error)) {
        unavailable = true
        notice = { text: 'この動画は評価を受け付けていません', failed: false }
      } else {
        notice = { text: ratingErrorMessage(error), failed: true }
      }
    } finally {
      sending = false
    }
  }
</script>

<button
  class="like"
  aria-label="高評価"
  aria-pressed={liked}
  disabled={sending || unavailable}
  onclick={toggle}
>
  <Icon>
    <path
      fill={liked ? 'currentColor' : 'none'}
      d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"
    />
  </Icon>
</button>

<style lang="sass">
.like
	flex: none
	display: inline-flex
	align-items: center
	justify-content: center
	width: 40px
	height: 40px
	background: transparent
	color: var(--c-text-sub)
	border: 0
	border-radius: var(--radius-sm)
	font-size: var(--fs-xl)
	cursor: pointer

	&:hover:not(:disabled)
		background: var(--c-overlay-2)
		color: var(--c-text)

	// Filled and accent when on: the shape carries the state, the hue only helps.
	// A touch screen keeps :hover after the tap, so the on colour must outrank it.
	&[aria-pressed="true"], &[aria-pressed="true"]:hover:not(:disabled)
		color: var(--c-accent)

	&:disabled
		cursor: default
		opacity: 0.5
</style>
