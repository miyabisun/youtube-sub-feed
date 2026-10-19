<script>
  import { relativeTime } from '$lib/relative-time.js'
  import { link } from '$lib/router.svelte.js'
  import Icon from '$lib/components/Icon.svelte'

  let { post, onhide = null, onrestore = null } = $props()

  let pending = $state(false)

  async function changeVisibility() {
    if (pending) return
    pending = true
    try {
      await (onrestore ?? onhide)(post.id)
    } finally {
      pending = false
    }
  }

  let body = $state(null)
  let clamped = $state(false)
  let expanded = $state(false)

  $effect(() => {
    if (!body || expanded) return
    const measure = () => (clamped = body.scrollHeight > body.clientHeight + 1)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(body)
    return () => observer.disconnect()
  })
</script>

<article class="post-card">
  <div class="meta">
    {#if post.channel_thumbnail}
      <img class="avatar" src={post.channel_thumbnail} alt="" loading="lazy" />
    {/if}
    <a class="channel" href={link(`/channel/${post.channel_id}`)}>{post.channel_title}</a>
    <span class="time">{relativeTime(post.published_at)}</span>
    {#if onhide || onrestore}
      <button
        class="post-state"
        class:restore={!!onrestore}
        disabled={pending}
        onclick={changeVisibility}
      >
        {onrestore ? '復元' : 'もう見た'}
      </button>
    {/if}
  </div>
  {#if post.content}
    <p class="content" class:expanded bind:this={body}>{post.content}</p>
    {#if clamped || expanded}
      <button class="more" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
        {expanded ? '閉じる' : '続きを読む'}
      </button>
    {/if}
  {/if}
  {#if post.images.length === 1}
    <img class="image single" src={post.images[0]} alt="投稿の画像" loading="lazy" />
  {:else if post.images.length > 1}
    <div class="images">
      {#each post.images as src, i (src)}
        <img class="image" {src} alt={`投稿の画像 ${i + 1}/${post.images.length}`} loading="lazy" />
      {/each}
    </div>
  {/if}
  <div class="actions">
    <a
      class="reply"
      href={`https://www.youtube.com/post/${encodeURIComponent(post.id)}`}
      target="_blank"
      rel="noopener"
    >
      <Icon><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /></Icon>
      返信
    </a>
  </div>
</article>

<style lang="sass">
.post-card
	display: flex
	flex-direction: column
	gap: var(--sp-2)
	padding: var(--sp-3)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)
	min-width: 0

.meta
	display: flex
	align-items: center
	gap: var(--sp-2)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	min-width: 0

.avatar
	width: 24px
	height: 24px
	border-radius: var(--radius-full)
	flex-shrink: 0

.channel
	color: var(--c-text-sub)
	overflow: hidden
	text-overflow: ellipsis
	white-space: nowrap

	&:hover
		color: var(--c-text)
		text-decoration: underline

.time
	color: var(--c-text-muted)
	flex-shrink: 0

.post-state
	position: relative
	flex-shrink: 0
	margin-left: auto
	padding: var(--sp-1) var(--sp-3)
	background: var(--c-overlay-1)
	color: var(--c-text-sub)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-xs)
	cursor: pointer
	opacity: 0.7

	&::before
		content: ''
		position: absolute
		top: -12px
		right: -8px
		bottom: -12px
		left: -8px

	&:hover
		background: var(--c-danger-bg)
		color: var(--c-danger)
		border-color: var(--c-danger-border)
		opacity: 1

	&.restore:hover
		background: var(--c-accent-bg)
		color: var(--c-text)
		border-color: var(--c-accent-border)

	&:disabled
		opacity: 0.5
		cursor: wait

.content
	margin: 0
	white-space: pre-wrap
	overflow-wrap: anywhere
	line-height: 1.6
	display: -webkit-box
	-webkit-line-clamp: 6
	-webkit-box-orient: vertical
	overflow: hidden

	&.expanded
		display: block
		-webkit-line-clamp: unset

.more
	align-self: flex-start
	padding: 0
	background: none
	border: none
	color: var(--c-link)
	font-size: var(--fs-sm)
	cursor: pointer

.image
	display: block
	width: 100%
	border-radius: var(--radius-md)
	background: var(--c-overlay-1)
	object-fit: cover

.single
	max-height: 360px
	object-fit: contain

.images
	display: flex
	gap: var(--sp-2)
	overflow-x: auto
	scroll-snap-type: x mandatory

	.image
		flex: 0 0 85%
		max-width: 360px
		aspect-ratio: 1
		scroll-snap-align: start

@media (min-width: 768px)
	.content:not(.expanded)
		-webkit-line-clamp: 4

	.single
		max-height: 280px

	.images .image
		max-width: 280px

.actions
	display: flex
	justify-content: flex-end

.reply
	display: inline-flex
	align-items: center
	gap: var(--sp-1)
	padding: var(--sp-1) var(--sp-3)
	color: var(--c-text)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-sm)

	&:hover
		background: var(--c-overlay-2)
</style>
