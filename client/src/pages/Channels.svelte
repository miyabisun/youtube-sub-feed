<script>
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import Spinner from '$lib/components/Spinner.svelte';
	import { navigate } from '$lib/router.svelte.js';

	let channels = $state([]);
	let loading = $state(true);
	let search = $state('');

	let filtered = $derived(
		search
			? channels.filter((c) => c.title.toLowerCase().includes(search.toLowerCase()))
			: channels
	);

	async function loadChannels() {
		try {
			channels = await fetcher(`${config.path.api}/channels`);
		} catch {}
		loading = false;
	}

	loadChannels();
</script>

<div class="channels-page">
	<div class="search-bar">
		<input type="text" placeholder="チャンネル名で検索" bind:value={search} />
	</div>

	{#if loading}
		<Spinner />
	{:else}
		<div class="channel-list">
			{#each filtered as ch (ch.id)}
				<div class="channel-item" onclick={() => navigate(`/channel/${ch.id}`)} onkeydown={(e) => e.key === 'Enter' && navigate(`/channel/${ch.id}`)} role="button" tabindex="0">
					{#if ch.thumbnail_url}
						<img class="avatar" src={ch.thumbnail_url} alt="" loading="lazy" />
					{:else}
						<div class="avatar placeholder"></div>
					{/if}
					<div class="channel-info">
						<div class="channel-name">{ch.title}</div>
						{#if ch.group_names}
							<div class="channel-groups">{ch.group_names}</div>
						{/if}
					</div>
					<a class="yt-link" href="https://www.youtube.com/channel/{ch.id}" target="_blank" rel="noopener" onclick={(e) => e.stopPropagation()}>YT</a>
				</div>
			{/each}
		</div>
	{/if}
</div>

<style lang="sass">
.channels-page
	padding: var(--sp-3) var(--sp-4)
	max-width: 640px
	margin: 0 auto

.search-bar
	margin-bottom: var(--sp-4)

	input
		width: 100%
		padding: var(--sp-3) var(--sp-4)
		background: var(--c-surface)
		border: 1px solid var(--c-border)
		border-radius: var(--radius-md)
		color: var(--c-text)
		font-size: var(--fs-md)

		&:focus
			outline: none
			border-color: var(--c-accent)

.channel-list
	display: flex
	flex-direction: column

.channel-item
	display: flex
	align-items: center
	gap: var(--sp-3)
	padding: var(--sp-3)
	color: inherit
	border-bottom: 1px solid var(--c-border)
	cursor: pointer

	&:hover
		background: var(--c-overlay-1)

.avatar
	width: 40px
	height: 40px
	border-radius: 50%
	flex-shrink: 0
	object-fit: cover

	&.placeholder
		background: var(--c-surface)

.channel-info
	min-width: 0
	flex: 1

.yt-link
	flex-shrink: 0
	padding: var(--sp-1) var(--sp-3)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	text-decoration: none
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	white-space: nowrap

	&:hover
		color: var(--c-accent)
		border-color: var(--c-accent-border)

.channel-name
	font-size: var(--fs-md)
	white-space: nowrap
	overflow: hidden
	text-overflow: ellipsis

.channel-groups
	font-size: var(--fs-xs)
	color: var(--c-text-muted)
	margin-top: 2px
</style>
