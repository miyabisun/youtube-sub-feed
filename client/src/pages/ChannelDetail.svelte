<script>
	import { untrack } from 'svelte';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import VideoCard from '$lib/components/VideoCard.svelte';
	import Spinner from '$lib/components/Spinner.svelte';
	import Toast from '$lib/components/Toast.svelte';
	import { swipeable } from '$lib/swipe.js';

	let { channelId } = $props();

	let channel = $state(null);
	let videos = $state([]);
	let loading = $state(true);
	let loadingMore = $state(false);
	let hasMore = $state(true);
	let toast = $state(null);
	let sentinel = $state(null);

	const LIMIT = 100;

	async function loadData(reset = false) {
		if (reset) {
			videos = [];
			hasMore = true;
			loading = true;
		} else {
			loadingMore = true;
		}

		try {
			if (reset) {
				const channels = await fetcher(`${config.path.api}/channels`);
				channel = channels.find((c) => c.id === channelId) || null;
			}
			const offset = videos.length;
			const data = await fetcher(`${config.path.api}/channels/${channelId}/videos?limit=${LIMIT}&offset=${offset}`);
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
			videos = videos.map((v) => v.id === id ? { ...v, is_hidden: 1 } : v);
			toast = { message: '非表示にしました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function unhideVideo(id) {
		try {
			await fetcher(`${config.path.api}/videos/${id}/unhide`, { method: 'PATCH' });
			videos = videos.map((v) => v.id === id ? { ...v, is_hidden: 0 } : v);
			toast = { message: '復元しました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function toggleSetting(field) {
		if (!channel) return;
		const newVal = channel[field] ? 0 : 1;
		try {
			await fetcher(`${config.path.api}/channels/${channelId}`, {
				method: 'PATCH',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ [field]: newVal }),
			});
			channel = { ...channel, [field]: newVal };
			toast = { message: '設定を更新しました', type: 'success' };
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	async function refreshChannel() {
		try {
			const result = await fetcher(`${config.path.api}/channels/${channelId}/refresh`, { method: 'POST' });
			toast = { message: `${result.newVideos}件の新着`, type: 'success' };
			loadData(true);
		} catch (e) {
			toast = { message: e.message, type: 'error' };
		}
	}

	$effect(() => {
		channelId;
		untrack(() => loadData(true));
	});

	$effect(() => {
		if (!sentinel) return;
		const observer = new IntersectionObserver((entries) => {
			if (entries[0].isIntersecting && hasMore && !loading && !loadingMore) loadData();
		}, { rootMargin: '200px' });
		observer.observe(sentinel);
		return () => observer.disconnect();
	});
</script>

<div class="channel-detail">
	{#if loading}
		<Spinner />
	{:else}
		{#if channel}
			<div class="channel-header">
				<div class="channel-name-row">
					<div class="channel-name">{channel.title}</div>
					<a class="youtube-link" href="https://www.youtube.com/channel/{channel.id}" target="_blank" rel="noopener">YouTube</a>
				</div>
				<div class="channel-settings">
					<label class="toggle">
						<input type="checkbox" checked={channel.show_livestreams} onchange={() => toggleSetting('show_livestreams')} />
						ライブ表示
					</label>
					<button class="refresh-btn" onclick={refreshChannel}>更新</button>
				</div>
			</div>
		{/if}

		<div class="video-list">
			{#each videos as video (video.id)}
				<div class="video-wrapper" class:hidden={video.is_hidden}>
					<div class="swipe-bg">{video.is_hidden ? '復元' : '非表示'}</div>
					<div class="video-item"
						use:swipeable={{
							onSwipeLeft: video.is_hidden ? null : () => hideVideo(video.id),
							onSwipeRight: video.is_hidden ? () => unhideVideo(video.id) : null,
						}}
					>
						{#if video.is_hidden}<div class="hidden-badge"></div>{/if}
						<VideoCard {video} />
						<div class="action-btns">
							{#if video.is_hidden}
								<button class="action-btn restore" onclick={() => unhideVideo(video.id)}>復元</button>
							{:else}
								<button class="action-btn hide" onclick={() => hideVideo(video.id)}>非表示</button>
							{/if}
						</div>
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
.channel-detail
	padding: var(--sp-3) var(--sp-4)
	max-width: 640px
	margin: 0 auto

.channel-header
	margin-bottom: var(--sp-4)
	padding-bottom: var(--sp-3)
	border-bottom: 1px solid var(--c-border)

.channel-name-row
	display: flex
	align-items: center
	gap: var(--sp-3)
	margin-bottom: var(--sp-3)

.channel-name
	font-size: var(--fs-lg)
	font-weight: bold

.youtube-link
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	text-decoration: none
	padding: var(--sp-1) var(--sp-3)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	white-space: nowrap

	&:hover
		color: var(--c-accent)
		border-color: var(--c-accent-border)

.channel-settings
	display: flex
	align-items: center
	gap: var(--sp-4)
	flex-wrap: wrap

.toggle
	display: flex
	align-items: center
	gap: var(--sp-2)
	font-size: var(--fs-sm)
	color: var(--c-text-sub)
	cursor: pointer

	input
		accent-color: var(--c-accent)

.refresh-btn
	padding: var(--sp-2) var(--sp-4)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	color: var(--c-text-sub)
	font-size: var(--fs-sm)
	cursor: pointer

	&:hover
		background: var(--c-overlay-2)

.video-list
	display: flex
	flex-direction: column
	gap: var(--sp-4)

.video-wrapper
	position: relative
	overflow: hidden
	border-radius: var(--radius-md)

	&.hidden
		opacity: 0.5

.video-item
	position: relative
	z-index: 1
	background: var(--c-bg)

.hidden-badge
	position: absolute
	top: var(--sp-2)
	right: var(--sp-2)
	width: 24px
	height: 24px
	z-index: 3
	background: rgba(0, 0, 0, 0.7)
	border-radius: 50%
	&::before, &::after
		content: ''
		position: absolute
	&::before
		width: 16px
		height: 16px
		border: 2px solid var(--c-danger)
		border-radius: 50%
		top: 2px
		left: 2px
	&::after
		width: 2px
		height: 16px
		background: var(--c-danger)
		top: 4px
		left: 11px
		transform: rotate(45deg)
		transform-origin: center

.action-btns
	display: none

.swipe-bg
	display: none

.sentinel
	height: 1px

@media (min-width: 600px)
	.action-btns
		display: block
		position: absolute
		top: var(--sp-2)
		right: var(--sp-2)
		z-index: 2
		opacity: 0

	.video-wrapper:hover .action-btns
		opacity: 1

	.action-btn
		padding: var(--sp-1) var(--sp-3)
		background: rgba(0, 0, 0, 0.7)
		border: 1px solid var(--c-border)
		border-radius: var(--radius-sm)
		font-size: var(--fs-xs)
		cursor: pointer

	.action-btn.hide
		color: var(--c-text-sub)
		&:hover
			color: var(--c-danger)
			border-color: var(--c-danger-border)

	.action-btn.restore
		color: var(--c-text-sub)
		&:hover
			color: var(--c-accent)
			border-color: var(--c-accent-border)

@media (min-width: 800px)
	.channel-detail
		max-width: none

	.video-list
		display: grid
		grid-template-columns: repeat(3, 1fr)

@media (max-width: 599px)
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

	.hidden .swipe-bg
		left: 0
		right: auto
		justify-content: flex-start
		padding-left: var(--sp-5)
		color: var(--c-accent)
</style>
