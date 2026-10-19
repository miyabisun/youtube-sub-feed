<script>
  import config from '$lib/config.js'
  import fetcher from '$lib/fetcher.js'
  import { relativeTime } from '$lib/relative-time.js'
  import VideoCard from '$lib/components/VideoCard.svelte'
  import PostCard from '$lib/components/PostCard.svelte'
  import Spinner from '$lib/components/Spinner.svelte'
  import Toast from '$lib/components/Toast.svelte'

  const LIMIT = 100

  let items = $state([])
  let loading = $state(true)
  let loadingMore = $state(false)
  let hasMore = $state(true)
  let toast = $state(null)
  let sentinel = $state(null)

  async function loadHistory() {
    if (loadingMore || !hasMore) return
    if (items.length > 0) loadingMore = true

    try {
      const data = await fetcher(`${config.path.api}/history?limit=${LIMIT}&offset=${items.length}`)
      items = [...items, ...data]
      hasMore = data.length === LIMIT
    } catch (e) {
      toast = { message: e.message, type: 'error' }
    } finally {
      loading = false
      loadingMore = false
    }
  }

  async function restorePost(id) {
    try {
      await fetcher(`${config.path.api}/posts/${encodeURIComponent(id)}/unhide`, {
        method: 'PATCH',
      })
      items = items.filter((item) => item.kind !== 'post' || item.id !== id)
      toast = { message: '復元しました', type: 'success' }
    } catch (e) {
      toast = { message: e.message, type: 'error' }
    }
  }

  loadHistory()

  $effect(() => {
    if (!sentinel) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasMore && !loadingMore) loadHistory()
      },
      { rootMargin: '200px' },
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  })
</script>

<section class="history" aria-labelledby="history-title">
  <header class="history-header">
    <h1 id="history-title">視聴履歴</h1>
    <p>「もう見た」にした動画・投稿</p>
  </header>

  {#if loading}
    <Spinner />
  {:else if items.length === 0}
    <p class="empty">視聴履歴はまだありません</p>
  {:else}
    <div class="video-list">
      {#each items as item (`${item.kind ?? 'video'}:${item.id}`)}
        <article class="history-item">
          {#if item.kind === 'post'}
            <PostCard post={item} onrestore={restorePost} />
          {:else}
            <VideoCard video={item} />
          {/if}
          {#if item.watched_at}
            <p class="watched-at">記録: {relativeTime(item.watched_at)}</p>
          {/if}
        </article>
      {/each}
    </div>
    {#if hasMore}
      <div bind:this={sentinel} class="sentinel">
        {#if loadingMore}<Spinner />{/if}
      </div>
    {/if}
  {/if}
</section>

{#if toast}
  {#key Date.now()}
    <Toast message={toast.message} type={toast.type} />
  {/key}
{/if}

<style lang="sass">
.history
	padding: var(--sp-4)
	max-width: 640px
	margin: 0 auto

.history-header
	margin-bottom: var(--sp-4)
	padding-bottom: var(--sp-3)
	border-bottom: 1px solid var(--c-border)

	h1
		margin: 0
		font-size: var(--fs-xl)
		font-weight: 600

	p
		margin: var(--sp-1) 0 0
		color: var(--c-text-muted)
		font-size: var(--fs-xs)

.video-list
	display: flex
	flex-direction: column
	gap: var(--sp-4)

.history-item
	min-width: 0

.watched-at
	margin: var(--sp-1) 0 0
	padding-top: var(--sp-1)
	border-top: 1px solid var(--c-border)
	color: var(--c-text-muted)
	font-size: var(--fs-xs)

.empty
	margin: 0
	padding: var(--sp-5)
	text-align: center
	color: var(--c-text-sub)

.sentinel
	height: 1px

@media (min-width: 768px)
	.history
		max-width: none

	.video-list
		display: grid
		grid-template-columns: repeat(3, minmax(0, 1fr))
		align-items: start
</style>
