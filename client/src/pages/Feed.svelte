<script>
  import { untrack } from 'svelte'
  import config from '$lib/config.js'
  import fetcher from '$lib/fetcher.js'
  import { getGroups, loadGroups } from '$lib/groups.svelte.js'
  import VideoCard from '$lib/components/VideoCard.svelte'
  import PostCard from '$lib/components/PostCard.svelte'
  import Spinner from '$lib/components/Spinner.svelte'
  import Toast from '$lib/components/Toast.svelte'
  import Icon from '$lib/components/Icon.svelte'
  import { swipeNav } from '$lib/swipe.js'
  import { navigate } from '$lib/router.svelte.js'
  import { startAutoReload } from '$lib/auto-reload.js'
  import { createVideoHider } from '$lib/video-watch.js'
  import { feedKinds, kindsParam } from '$lib/feed-kinds.svelte.js'

  let { groupId = null } = $props()
  let groups = $derived(getGroups())

  let kinds = $derived(kindsParam(feedKinds))
  let videos = $state([])
  let loading = $state(false)
  let loadingMore = $state(false)
  let hasMore = $state(true)
  let toast = $state(null)
  let sentinel = $state(null)
  let requestSeq = 0

  const LIMIT = 100

  async function loadVideos(reset = false) {
    if (!reset && (loading || loadingMore)) return
    const seq = ++requestSeq
    if (!kinds) {
      videos = []
      hasMore = false
      loading = false
      loadingMore = false
      return
    }

    if (reset) {
      loading = true
      loadingMore = false
    } else loadingMore = true

    try {
      const offset = reset ? 0 : videos.length
      let url = `${config.path.api}/feed?kinds=${kinds}&limit=${LIMIT}&offset=${offset}`
      if (groupId) url += `&group=${groupId}`
      const data = await fetcher(url)
      if (seq !== requestSeq) return
      videos = reset ? data : [...videos, ...data]
      hasMore = data.length === LIMIT
    } catch (e) {
      if (seq !== requestSeq) return
      toast = { message: e.message, type: 'error' }
    } finally {
      if (seq === requestSeq) {
        loading = false
        loadingMore = false
      }
    }
  }

  const hideVideo = createVideoHider({
    hideRequest: (id, options) => fetcher(`${config.path.api}/videos/${id}/hide`, options),
    onHidden: (id) => (videos = videos.filter((v) => v.kind !== 'video' || v.id !== id)),
    onSuccess: () => (toast = { message: '非表示にしました', type: 'success' }),
    onError: (error) => (toast = { message: error.message, type: 'error' }),
  })

  const hidePost = createVideoHider({
    hideRequest: (id, options) =>
      fetcher(`${config.path.api}/posts/${encodeURIComponent(id)}/hide`, options),
    onHidden: (id) => (videos = videos.filter((v) => v.kind !== 'post' || v.id !== id)),
    onSuccess: () => (toast = { message: '非表示にしました', type: 'success' }),
    onError: (error) => (toast = { message: error.message, type: 'error' }),
  })

  let groupCycle = $derived([null, ...groups.map((g) => String(g.id))])

  function navigateGroup(delta) {
    const currentIndex = groupCycle.indexOf(groupId ?? null)
    const nextIndex = (currentIndex + delta + groupCycle.length) % groupCycle.length
    const next = groupCycle[nextIndex]
    navigate(next ? `/group/${next}` : '/')
  }

  loadGroups()

  $effect(() => {
    void groupId
    void kinds
    untrack(() => loadVideos(true))
  })

  $effect(() => startAutoReload(() => loadVideos(true)))

  $effect(() => {
    if (!sentinel) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasMore && !loading && !loadingMore) {
          loadVideos()
        }
      },
      { rootMargin: '200px' },
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  })
</script>

<div
  class="feed"
  use:swipeNav={{ onLeft: () => navigateGroup(1), onRight: () => navigateGroup(-1) }}
>
  <div class="kind-bar" role="group" aria-label="表示する種類">
    {#each [['video', '動画'], ['post', '投稿']] as [kind, label] (kind)}
      <button
        class="kind-btn"
        aria-pressed={feedKinds[kind]}
        onclick={() => (feedKinds[kind] = !feedKinds[kind])}
      >
        <Icon>
          {#if feedKinds[kind]}
            <polyline points="5 12 10 17 19 7" />
          {:else}
            <line x1="5" y1="12" x2="19" y2="12" />
          {/if}
        </Icon>
        {label}
      </button>
    {/each}
  </div>
  {#if !kinds}
    <p class="empty">「動画」か「投稿」をオンにすると表示されます</p>
  {:else if loading && videos.length === 0}
    <Spinner />
  {:else if videos.length === 0}
    <p class="empty">表示できる項目がありません</p>
  {:else}
    <div class="video-list">
      {#each videos as item (`${item.kind}:${item.id}`)}
        {#if item.kind === 'post'}
          <PostCard post={item} onhide={hidePost} />
        {:else}
          <div class="video-wrapper">
            <div class="video-item">
              <VideoCard video={item} onwatch={(id) => hideVideo(id, { silent: true })} />
              <button class="hide-btn" onclick={() => hideVideo(item.id)}>もう見た</button>
            </div>
          </div>
        {/if}
      {/each}
    </div>
    {#if hasMore}
      <div bind:this={sentinel} class="sentinel">
        {#if loadingMore}<Spinner />{/if}
      </div>
    {/if}
  {/if}
</div>

{#if toast}
  {#key Date.now()}
    <Toast message={toast.message} type={toast.type} />
  {/key}
{/if}

<style lang="sass">
.feed
	padding: var(--sp-3) var(--sp-4)
	max-width: 640px
	margin: 0 auto

.video-list
	display: flex
	flex-direction: column
	gap: var(--sp-4)

.video-wrapper
	position: relative
	overflow: hidden
	border-radius: var(--radius-md)

.video-item
	position: relative
	z-index: 1
	background: var(--c-bg)

.hide-btn
	display: block
	position: absolute
	top: var(--sp-2)
	right: var(--sp-2)
	padding: var(--sp-1) var(--sp-3)
	background: var(--c-scrim-strong)
	color: var(--c-text-sub)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-xs)
	cursor: pointer
	z-index: 2
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

.kind-bar
	display: flex
	gap: var(--sp-2)
	margin-bottom: var(--sp-3)

.kind-btn
	display: inline-flex
	align-items: center
	gap: var(--sp-1)
	padding: var(--sp-1) var(--sp-3)
	background: transparent
	color: var(--c-text-sub)
	border: 1px dashed var(--c-border)
	border-radius: var(--radius-full)
	font-size: var(--fs-sm)
	cursor: pointer

	&[aria-pressed="true"]
		background: var(--c-accent-bg)
		color: var(--c-text)
		border: 1px solid var(--c-accent-border)

	&:hover
		background: var(--c-overlay-2)

	&[aria-pressed="true"]:hover
		background: var(--c-accent-bg)

.empty
	text-align: center
	padding: var(--sp-5)
	color: var(--c-text-sub)

.sentinel
	height: 1px

@media (min-width: 768px)
	.feed
		max-width: none

	.video-list
		display: grid
		grid-template-columns: repeat(3, 1fr)
		align-items: start
</style>
