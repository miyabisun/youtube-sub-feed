<script>
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import VideoCard from '$lib/components/VideoCard.svelte';
	import Spinner from '$lib/components/Spinner.svelte';
	import Toast from '$lib/components/Toast.svelte';
	import { swipeable } from '$lib/swipe.js';

	let { groupId = null } = $props();

	let videos = $state([]);
	let loading = $state(false);
	let loadingMore = $state(false);
	let hasMore = $state(true);
	let toast = $state(null);
	let sentinel;

	const LIMIT = 100;

	async function loadVideos(reset = false) {
		if (reset) {
			videos = [];
			hasMore = true;
		}
		if (loading || loadingMore) return;

		if (reset) loading = true;
		else loadingMore = true;

		try {
			const offset = videos.length;
			let url = `${config.path.api}/feed?limit=${LIMIT}&offset=${offset}`;
			if (groupId) url += `&group=${groupId}`;
			const data = await fetcher(url);
			videos = [...videos, ...data];
			hasMore = data.length === LIMIT;
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		} finally {
			loading = false;
			loadingMore = false;
		}
	}

	async function hideVideo(id) {
		try {
			await fetcher(`${config.path.api}/videos/${id}/hide`, { method: 'PATCH' });
			videos = videos.filter((v) => v.id !== id);
			toast = { message: '非表示にしました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	$effect(() => {
		groupId;
		loadVideos(true);
	});

	$effect(() => {
		if (!sentinel) return;
		const observer = new IntersectionObserver((entries) => {
			if (entries[0].isIntersecting && hasMore && !loading && !loadingMore) {
				loadVideos();
			}
		}, { rootMargin: '200px' });
		observer.observe(sentinel);
		return () => observer.disconnect();
	});
</script>

<div class="feed">
	{#if loading}
		<Spinner />
	{:else if videos.length === 0}
		<p class="empty">動画がありません</p>
	{:else}
		<div class="video-list">
			{#each videos as video (video.id)}
				<div class="video-wrapper">
					<div class="swipe-bg">非表示</div>
					<div class="video-item" use:swipeable={{ onSwipeLeft: () => hideVideo(video.id) }}>
						<VideoCard {video} />
						<button class="hide-btn" onclick={() => hideVideo(video.id)}>非表示</button>
					</div>
				</div>
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
	display: none
	position: absolute
	top: var(--sp-2)
	right: var(--sp-2)
	padding: var(--sp-1) var(--sp-3)
	background: rgba(0, 0, 0, 0.7)
	color: var(--c-text-sub)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-xs)
	cursor: pointer
	z-index: 2

	&:hover
		background: var(--c-danger-bg)
		color: var(--c-danger)
		border-color: var(--c-danger-border)

.swipe-bg
	display: none

.empty
	text-align: center
	padding: var(--sp-6)
	color: var(--c-text-sub)

.sentinel
	height: 1px

@media (min-width: 600px)
	.hide-btn
		display: block
		opacity: 0

	.video-wrapper:hover .hide-btn
		opacity: 1

@media (max-width: 599px)
	.video-wrapper
		overflow: hidden

	.video-item
		background: var(--c-bg)

	.swipe-bg
		display: flex
		align-items: center
		justify-content: flex-end
		padding-right: var(--sp-5)
		position: absolute
		right: 0
		top: 0
		bottom: 0
		width: 80px
		color: var(--c-danger)
		font-weight: bold
		font-size: var(--fs-sm)
		opacity: 0
</style>
