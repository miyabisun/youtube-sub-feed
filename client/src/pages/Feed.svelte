<script>
	import { untrack } from 'svelte';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import VideoCard from '$lib/components/VideoCard.svelte';
	import Spinner from '$lib/components/Spinner.svelte';
	import Toast from '$lib/components/Toast.svelte';
	import { swipeNav } from '$lib/swipe.js';
	import { navigate } from '$lib/router.svelte.js';

	let { groupId = null } = $props();
	let groups = $state([]);

	let videos = $state([]);
	let loading = $state(false);
	let loadingMore = $state(false);
	let hasMore = $state(true);
	let toast = $state(null);
	let sentinel = $state(null);

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

	const PLAY_ALL_LIMIT = 20;

	function openPlayAll() {
		const ids = videos.filter(v => !v.is_short).slice(0, PLAY_ALL_LIMIT).map(v => v.id).join(',');
		window.open(`https://www.youtube.com/watch_videos?video_ids=${ids}`);
	}

	async function loadGroups() {
		try {
			groups = await fetcher(`${config.path.api}/groups`);
		} catch (_) {}
	}

	function swipeLeft() {
		const cycle = [null, ...groups.map((g) => String(g.id))];
		const currentIndex = cycle.indexOf(groupId ?? null);
		const nextIndex = (currentIndex + 1) % cycle.length;
		const next = cycle[nextIndex];
		navigate(next ? `/group/${next}` : '/');
	}

	function swipeRight() {
		const cycle = [null, ...groups.map((g) => String(g.id))];
		const currentIndex = cycle.indexOf(groupId ?? null);
		const prevIndex = (currentIndex - 1 + cycle.length) % cycle.length;
		const prev = cycle[prevIndex];
		navigate(prev ? `/group/${prev}` : '/');
	}

	$effect(() => {
		untrack(() => loadGroups());
	});

	$effect(() => {
		groupId;
		untrack(() => loadVideos(true));
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

<div class="feed" use:swipeNav={{ onLeft: swipeLeft, onRight: swipeRight }}>
	{#if loading}
		<Spinner />
	{:else if videos.length === 0}
		<p class="empty">動画がありません</p>
	{:else}
		{@const playableCount = videos.filter(v => !v.is_short).length}
		{#if playableCount > 0}
			<button class="play-all-btn" onclick={openPlayAll}>
				▶ 連続再生 ({Math.min(playableCount, PLAY_ALL_LIMIT)}本)
			</button>
		{/if}
		<div class="video-list">
			{#each videos as video (video.id)}
				<div class="video-wrapper">
					<div class="video-item">
						<VideoCard {video} />
						<button class="hide-btn" onclick={() => hideVideo(video.id)}>もう見た</button>
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
	display: block
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

.play-all-btn
	display: block
	margin-bottom: var(--sp-3)
	padding: var(--sp-2) var(--sp-4)
	background: var(--c-surface)
	color: var(--c-text)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	font-size: var(--fs-sm)
	cursor: pointer

	&:hover
		opacity: 0.8

.empty
	text-align: center
	padding: var(--sp-6)
	color: var(--c-text-sub)

.sentinel
	height: 1px

@media (max-width: 599px)
	.hide-btn
		opacity: 0.7

@media (min-width: 600px)
	.hide-btn
		opacity: 0

	.video-wrapper:hover .hide-btn
		opacity: 0.7

	.video-wrapper:hover .hide-btn:hover
		opacity: 1

@media (min-width: 800px)
	.feed
		max-width: none

	.video-list
		display: grid
		grid-template-columns: repeat(3, 1fr)
</style>
