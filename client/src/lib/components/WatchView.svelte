<script>
  import { closeWatch, link } from '$lib/router.svelte.js'
  import { embedUrl, youtubeUrl } from '$lib/youtube-url.js'
  import { observePlayer } from '$lib/youtube-player.js'
  import { relativeTime } from '$lib/relative-time.js'
  import Icon from '$lib/components/Icon.svelte'
  import Comments from '$lib/components/Comments.svelte'

  let { id, video = null } = $props()

  let dialog = $state(null)
  let frame = $state(null)
  let playerTitle = $state('')
  let playerFailed = $state(false)

  let title = $derived(video?.title || playerTitle)
  let externalUrl = $derived(youtubeUrl(video ?? { id }))

  let unmounting = false

  $effect(() => {
    dialog.showModal()
    return () => {
      unmounting = true
      dialog.close()
    }
  })

  $effect(() =>
    observePlayer(frame, {
      // The codes do not tell a blocked embed from a removed video reliably, and
      // the way out is the same for all of them.
      onError: () => (playerFailed = true),
      onTitle: (value) => (playerTitle = value || playerTitle),
    }),
  )

  // Esc and the Android back gesture ask the dialog to close; closing is the
  // router's job so the URL and the dialog never disagree.
  function handleCancel(event) {
    event.preventDefault()
    closeWatch()
  }

  // The browser may close the dialog without honouring the cancel above.
  function handleClose() {
    if (!unmounting) closeWatch()
  }
</script>

<dialog
  bind:this={dialog}
  class="watch"
  class:short={video?.is_short}
  aria-label={title || '動画'}
  oncancel={handleCancel}
  onclose={handleClose}
>
  <header class="bar">
    <button class="icon-btn" onclick={closeWatch} aria-label="閉じる">
      <Icon>
        <line x1="19" y1="12" x2="5" y2="12" />
        <polyline points="12 19 5 12 12 5" />
      </Icon>
    </button>
    <h1 class="title">{title}</h1>
    <a
      class="icon-btn"
      href={externalUrl}
      target="_blank"
      rel="noopener"
      aria-label="YouTube で開く"
    >
      <Icon>
        <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
        <polyline points="15 3 21 3 21 9" />
        <line x1="10" y1="14" x2="21" y2="3" />
      </Icon>
    </a>
  </header>

  <div class="stage">
    <div class="player">
      <iframe
        bind:this={frame}
        src={embedUrl(id, window.location.origin)}
        title={title || '動画プレイヤー'}
        allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
        allowfullscreen
        referrerpolicy="strict-origin-when-cross-origin"
      ></iframe>
    </div>
  </div>

  <div class="body">
    {#if playerFailed}
      <p class="player-error">
        この動画はここでは再生できません
        <a href={externalUrl} target="_blank" rel="noopener">YouTube で開く</a>
      </p>
    {/if}
    {#if video?.channel_title}
      <p class="meta">
        <a class="channel" href={link(`/channel/${video.channel_id}`)}>{video.channel_title}</a>
        {#if video.published_at}<span>{relativeTime(video.published_at)}</span>{/if}
      </p>
    {/if}
    <Comments videoId={id} />
  </div>
</dialog>

<style lang="sass">
// A full-screen reader, not the centred 80dvh modal: the player is the content.
.watch
	width: 100vw
	max-width: none
	height: 100dvh
	max-height: none
	margin: 0
	padding: 0
	border: 0
	background: var(--c-bg)
	color: var(--c-text)
	overflow-y: auto

	&::backdrop
		background: var(--c-bg)

.bar
	display: flex
	align-items: center
	gap: var(--sp-1)
	height: 48px
	padding: 0 var(--sp-1)
	background: var(--c-bg)
	border-bottom: 1px solid var(--c-border)

.icon-btn
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

	&:hover
		background: var(--c-overlay-2)
		color: var(--c-text)

.title
	flex: 1
	min-width: 0
	margin: 0
	font-size: var(--fs-lg)
	font-weight: normal
	white-space: nowrap
	overflow: hidden
	text-overflow: ellipsis

// The video always gets the most the screen allows: the full width when the
// screen is upright, the full height under the bar when it lies on its side.
.stage
	background: #000

.player
	width: min(100%, calc((100dvh - 48px) * 16 / 9))
	aspect-ratio: 16 / 9
	margin: 0 auto

	iframe
		display: block
		width: 100%
		height: 100%
		border: 0

// Portrait video: as tall as the screen allows, leaving a strip that shows
// there is more below.
.short .player
	width: auto
	height: calc(100dvh - 48px - 96px)
	max-width: 100%
	aspect-ratio: 9 / 16

.body
	max-width: 720px
	margin: 0 auto
	padding: var(--sp-3) var(--sp-3) var(--sp-5)

.player-error
	display: flex
	flex-wrap: wrap
	gap: 0 var(--sp-3)
	margin: 0 0 var(--sp-3)
	padding: var(--sp-2) var(--sp-3)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)
	font-size: var(--fs-sm)

.meta
	display: flex
	align-items: center
	gap: var(--sp-3)
	margin: 0 0 var(--sp-3)
	font-size: var(--fs-sm)
	color: var(--c-text-muted)

.meta span
	flex: none

.channel
	min-width: 0
	overflow: hidden
	text-overflow: ellipsis
	white-space: nowrap
	color: var(--c-text-sub)

	&:hover
		color: var(--c-text)
		text-decoration: underline

// Upright screen, landscape video: the bar and the video stay while the
// comments scroll beneath them.
@media (orientation: portrait)
	.watch:not(.short) .bar
		position: sticky
		top: 0
		z-index: 2

	.watch:not(.short) .stage
		position: sticky
		top: 48px
		z-index: 1

// Screen on its side: the video and the comments stand side by side, so the
// way back, the comments and the comment box are all on screen with the video.
// The video keeps everything except the narrowest column comments can use.
@media (orientation: landscape)
	.watch[open]
		display: grid
		grid-template-columns: auto minmax(320px, 1fr)
		grid-template-rows: 48px minmax(0, 1fr)
		overflow: hidden

	.bar
		grid-column: 1 / -1

	.stage
		display: grid
		place-items: center

	.player
		width: min(100vw - 320px, calc((100dvh - 48px) * 16 / 9))

	.short .player
		height: calc(100dvh - 48px)

	.body
		width: 100%
		overflow-y: auto
</style>
