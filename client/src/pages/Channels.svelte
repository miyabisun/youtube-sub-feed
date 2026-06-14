<script>
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import Spinner from '$lib/components/Spinner.svelte';
	import Toast from '$lib/components/Toast.svelte';
	import { navigate } from '$lib/router.svelte.js';

	let channels = $state([]);
	let loading = $state(true);
	let search = $state('');
	let toast = $state(null);

	// Manual channel add
	let addChannelId = $state('');
	let addTitle = $state('');
	let adding = $state(false);
	let addError = $state('');

	// Pending delete confirmation
	let pendingDeleteId = $state(null);
	let deleting = $state(false);

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

	/**
	 * Validate a YouTube channel ID.
	 * A valid ID is exactly 24 characters: "UC" + 22 base64url chars ([A-Za-z0-9_-]).
	 * Returns an error message string or null if valid.
	 */
	function validateChannelId(id) {
		if (!id) return 'チャンネルIDを入力してください';
		if (id.length !== 24) return `チャンネルID は24文字でなければなりません (UC + 22文字の base64url)。現在 ${id.length} 文字`;
		if (!id.startsWith('UC')) return 'チャンネルID は UC で始まる必要があります';
		if (!/^[A-Za-z0-9_-]{22}$/.test(id.slice(2))) return 'チャンネルID の UC 以降は英数字・アンダースコア・ハイフンのみ使えます';
		return null;
	}

	async function addChannel() {
		const channelId = addChannelId.trim();
		const title = addTitle.trim();
		addError = '';

		const validationError = validateChannelId(channelId);
		if (validationError) {
			addError = validationError;
			return;
		}

		adding = true;
		try {
			await fetcher(`${config.path.api}/channels`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ channel_id: channelId, title: title || channelId }),
			});
			addChannelId = '';
			addTitle = '';
			toast = { message: `チャンネルを追加しました: ${channelId}`, type: 'success' };
			await loadChannels();
		} catch (e) {
			addError = `追加に失敗しました: ${e.message}`;
		} finally {
			adding = false;
		}
	}

	function confirmDelete(channelId) {
		pendingDeleteId = channelId;
	}

	function cancelDelete() {
		pendingDeleteId = null;
	}

	async function removeChannel() {
		if (!pendingDeleteId || deleting) return;
		const id = pendingDeleteId;
		deleting = true;
		try {
			const res = await fetch(`${config.path.api}/channels/${id}`, { method: 'DELETE' });
			if (res.status === 401) {
				navigate('/login');
				return;
			}
			if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
			toast = { message: `チャンネルを削除しました`, type: 'success' };
			pendingDeleteId = null;
			await loadChannels();
		} catch (e) {
			toast = { message: `削除に失敗しました: ${e.message}`, type: 'error' };
		} finally {
			deleting = false;
		}
	}
</script>

<div class="channels-page">
	<!-- Manual channel add form -->
	<section class="add-section">
		<h2 class="section-title">チャンネルを手動追加</h2>
		<div class="add-form">
			<input
				type="text"
				class="add-input"
				placeholder="チャンネルID (UCxxxxxxxx)"
				bind:value={addChannelId}
				onkeydown={(e) => e.key === 'Enter' && addChannel()}
			/>
			<input
				type="text"
				class="add-input"
				placeholder="表示名 (省略可)"
				bind:value={addTitle}
				onkeydown={(e) => e.key === 'Enter' && addChannel()}
			/>
			<button class="add-button" onclick={addChannel} disabled={adding}>
				{adding ? '追加中...' : '追加'}
			</button>
		</div>
		{#if addError}
			<p class="add-error">{addError}</p>
		{/if}
	</section>

	<div class="search-bar">
		<input type="text" placeholder="チャンネル名で検索" bind:value={search} />
	</div>

	{#if loading}
		<Spinner />
	{:else}
		<div class="channel-list">
			{#each filtered as ch (ch.id)}
				<div class="channel-item">
					<div
						class="channel-clickable"
						onclick={() => navigate(`/channel/${ch.id}`)}
						onkeydown={(e) => e.key === 'Enter' && navigate(`/channel/${ch.id}`)}
						role="button"
						tabindex="0"
					>
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
					</div>
					<div class="channel-actions">
						<a class="yt-link" href="https://www.youtube.com/channel/{ch.id}" target="_blank" rel="noopener" onclick={(e) => e.stopPropagation()}>YT</a>
						{#if pendingDeleteId === ch.id}
							<button class="delete-confirm" onclick={removeChannel} disabled={deleting}>
								{deleting ? '削除中...' : '削除確認'}
							</button>
							<button class="delete-cancel" onclick={cancelDelete}>キャンセル</button>
						{:else}
							<button class="delete-button" onclick={() => confirmDelete(ch.id)} title="チャンネルを削除">✕</button>
						{/if}
					</div>
				</div>
			{/each}
		</div>
	{/if}
</div>

{#if toast}
	{#key Date.now()}
		<Toast message={toast.message} type={toast.type} />
	{/key}
{/if}

<style lang="sass">
.channels-page
	padding: var(--sp-3) var(--sp-4)
	max-width: 640px
	margin: 0 auto

.section-title
	font-size: var(--fs-sm)
	color: var(--c-text-sub)
	margin: 0 0 var(--sp-2)
	font-weight: 600

.add-section
	margin-bottom: var(--sp-4)
	padding: var(--sp-4)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)

.add-form
	display: flex
	gap: var(--sp-2)
	flex-wrap: wrap

.add-input
	flex: 1
	min-width: 160px
	padding: var(--sp-2) var(--sp-3)
	background: var(--c-bg)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	color: var(--c-text)
	font-size: var(--fs-sm)

	&:focus
		outline: none
		border-color: var(--c-accent)

.add-button
	padding: var(--sp-2) var(--sp-4)
	background: var(--c-accent)
	color: #fff
	border: none
	border-radius: var(--radius-sm)
	font-size: var(--fs-sm)
	cursor: pointer
	white-space: nowrap

	&:hover:not(:disabled)
		opacity: 0.85

	&:disabled
		opacity: 0.5
		cursor: wait

.add-error
	margin: var(--sp-2) 0 0
	font-size: var(--fs-xs)
	color: var(--c-error, #e53e3e)

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
	border-bottom: 1px solid var(--c-border)

	&:hover
		background: var(--c-overlay-1)

.channel-clickable
	display: flex
	align-items: center
	gap: var(--sp-3)
	padding: var(--sp-3)
	flex: 1
	min-width: 0
	cursor: pointer
	color: inherit

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

.channel-actions
	display: flex
	align-items: center
	gap: var(--sp-2)
	padding: var(--sp-2) var(--sp-3)
	flex-shrink: 0

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

.delete-button
	padding: var(--sp-1) var(--sp-2)
	font-size: var(--fs-xs)
	color: var(--c-text-muted)
	background: none
	border: 1px solid transparent
	border-radius: var(--radius-sm)
	cursor: pointer
	line-height: 1

	&:hover
		color: var(--c-error, #e53e3e)
		border-color: var(--c-error, #e53e3e)

.delete-confirm
	padding: var(--sp-1) var(--sp-2)
	font-size: var(--fs-xs)
	color: #fff
	background: var(--c-error, #e53e3e)
	border: none
	border-radius: var(--radius-sm)
	cursor: pointer
	white-space: nowrap

	&:disabled
		opacity: 0.5
		cursor: wait

.delete-cancel
	padding: var(--sp-1) var(--sp-2)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)
	background: none
	border: 1px solid var(--c-border)
	border-radius: var(--radius-sm)
	cursor: pointer
	white-space: nowrap

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
