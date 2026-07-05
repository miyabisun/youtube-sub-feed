<script>
	import { router, link, navigate } from '$lib/router.svelte.js';
	import { getGroups, loadGroups } from '$lib/groups.svelte.js';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import Toast from '$lib/components/Toast.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { toUserMessage } from '$lib/sync-error-message.js';

	let groups = $derived(getGroups());
	let menuOpen = $state(false);
	let syncing = $state(false);
	let toast = $state(null);

	loadGroups();

	let selectValue = $derived(router.index === 1 ? router.params.id : '');

	function isActive(href) {
		if (href === '/') return router.index === 0;
		if (href.startsWith('/group/')) return router.index === 1 && router.params.id === href.split('/')[2];
		if (href === '/channels') return router.index === 2;
		if (href === '/settings') return router.index === 5;
		return false;
	}

	function onGroupSelect(e) {
		const value = e.target.value;
		navigate(value ? `/group/${value}` : '/');
	}

	function toggleMenu() {
		menuOpen = !menuOpen;
	}

	function closeMenu() {
		menuOpen = false;
	}

	/**
	 * Browser-side GIS sync flow:
	 *   1. Use Google Identity Services token client to get a short-lived access_token
	 *      (online-only, no refresh_token requested).
	 *   2. Fetch YouTube Subscriptions.list (all pages) directly from the browser using
	 *      the access_token as Bearer (CORS-enabled, confirmed).
	 *   3. POST /api/channels/sync with the collected channel_ids and metadata.
	 *   4. Discard the token — it is never sent to or stored on the server.
	 */
	async function syncChannels() {
		if (syncing) return;
		syncing = true;
		closeMenu();

		const clientId = config.gisClientId;
		if (!clientId) {
			toast = { message: 'GIS_CLIENT_ID が設定されていません', type: 'error' };
			syncing = false;
			return;
		}

		try {
			const accessToken = await getGisToken(clientId);
			const { channelIds, meta } = await fetchAllSubscriptions(accessToken);

			if (channelIds.length === 0) {
				const ok = confirm(
					'取得された登録チャンネルが0件です。\n' +
					'API の一時的な異常やスコープ取得直後の空応答の可能性があります。\n' +
					'同期を続行すると登録済みチャンネルが全て削除される可能性があります。\n\n' +
					'続行しますか？'
				);
				if (!ok) {
					toast = { message: 'チャンネル同期をキャンセルしました', type: 'error' };
					return;
				}
			}

			const result = await fetcher(`${config.path.api}/channels/sync`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ channel_ids: channelIds, meta }),
			});
			const added = result?.added ?? 0;
			const removed = result?.removed ?? 0;
			toast = { message: `チャンネル同期完了 (追加: ${added}, 削除: ${removed})`, type: 'success' };
		} catch (e) {
			console.error('[sync] channel sync failed:', e);
			toast = { message: toUserMessage(e), type: 'error' };
		} finally {
			syncing = false;
		}
	}

	/**
	 * Request a short-lived online access_token via GIS token client.
	 * Does NOT request offline access — no refresh_token is ever issued.
	 * Returns the access_token string.
	 */
	function getGisToken(clientId) {
		return new Promise((resolve, reject) => {
			if (!window.google?.accounts?.oauth2) {
				reject(new Error('GIS SDK がロードされていません。ページをリロードしてください。'));
				return;
			}
			const client = window.google.accounts.oauth2.initTokenClient({
				client_id: clientId,
				scope: 'https://www.googleapis.com/auth/youtube.readonly',
				callback: (response) => {
					if (response.error) {
						reject(new Error(response.error_description || response.error));
					} else {
						resolve(response.access_token);
					}
				},
				error_callback: (err) => {
					reject(new Error(err?.message || 'token request cancelled'));
				},
			});
			client.requestAccessToken();
		});
	}

	/**
	 * Fetch all pages of YouTube Subscriptions.list using the provided access_token.
	 * YouTube Data API is CORS-enabled: Bearer token works from the browser.
	 * Returns { channelIds: string[], meta: Record<string, { title, thumbnail_url }> }.
	 */
	async function fetchAllSubscriptions(accessToken) {
		const channelIds = [];
		const meta = {};
		let pageToken = null;

		do {
			const url = new URL('https://www.googleapis.com/youtube/v3/subscriptions');
			url.searchParams.set('part', 'snippet');
			url.searchParams.set('mine', 'true');
			url.searchParams.set('maxResults', '50');
			if (pageToken) url.searchParams.set('pageToken', pageToken);

			const res = await fetch(url.toString(), {
				headers: { Authorization: `Bearer ${accessToken}` },
			});

			if (!res.ok) {
				const body = await res.json().catch(() => ({}));
				throw new Error(`YouTube API error: ${body?.error?.message || res.status}`);
			}

			const data = await res.json();
			for (const item of data.items ?? []) {
				const channelId = item.snippet?.resourceId?.channelId;
				if (!channelId) continue;
				channelIds.push(channelId);
				meta[channelId] = {
					title: item.snippet?.title ?? channelId,
					thumbnail_url: item.snippet?.thumbnails?.default?.url ?? null,
				};
			}
			pageToken = data.nextPageToken ?? null;
		} while (pageToken);

		return { channelIds, meta };
	}
</script>

<header>
	<select class="group-select" value={selectValue} onchange={onGroupSelect}>
		<option value="">すべて</option>
		{#each groups as group}
			<option value={String(group.id)}>{group.name}</option>
		{/each}
	</select>
	<nav class="nav-tabs">
		<a class="nav-item" class:active={isActive('/')} href={link('/')}>すべて</a>
		{#each groups as group}
			<a class="nav-item" class:active={isActive(`/group/${group.id}`)} href={link(`/group/${group.id}`)}>{group.name}</a>
		{/each}
	</nav>
	<div class="menu-wrapper">
		<button class="menu-button" onclick={toggleMenu} aria-label="メニュー">
			<Icon>
				<line x1="3" y1="6" x2="21" y2="6" />
				<line x1="3" y1="12" x2="21" y2="12" />
				<line x1="3" y1="18" x2="21" y2="18" />
			</Icon>
		</button>
		{#if menuOpen}
			<button class="menu-overlay" onclick={closeMenu} aria-label="メニューを閉じる"></button>
			<nav class="menu-dropdown">
				<a class="menu-item" class:active={isActive('/channels')} href={link('/channels')} onclick={closeMenu}>チャンネル</a>
				<a class="menu-item" class:active={isActive('/settings')} href={link('/settings')} onclick={closeMenu}>グループ管理</a>
				<button class="menu-item menu-action" onclick={syncChannels} disabled={syncing}>
					{syncing ? '同期中...' : 'チャンネル同期 (YouTube)'}
				</button>
			</nav>
		{/if}
	</div>
</header>

{#if toast}
	{#key Date.now()}
		<Toast message={toast.message} type={toast.type} />
	{/key}
{/if}

<style lang="sass">
header
	display: flex
	align-items: center
	justify-content: space-between
	border-bottom: 1px solid var(--c-border)
	background: var(--c-bg)
	position: relative
	z-index: 10

.group-select
	display: none
	flex: 1
	min-width: 0
	margin: var(--sp-2) var(--sp-3)
	padding: var(--sp-3) var(--sp-4)
	background: var(--c-surface)
	color: var(--c-text)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-md)
	font-size: var(--fs-sm)

	&:focus
		border-color: var(--c-accent)

.nav-tabs
	display: flex
	align-items: center
	overflow-x: auto
	flex: 1
	min-width: 0

	&::-webkit-scrollbar
		display: none

.nav-item
	padding: var(--sp-3) var(--sp-4)
	color: var(--c-text-sub)
	text-decoration: none
	font-size: var(--fs-sm)
	border-bottom: 2px solid transparent
	margin-bottom: -1px
	white-space: nowrap
	flex-shrink: 0

	&:hover
		color: var(--c-text)
		background: var(--c-overlay-1)

	&.active
		color: var(--c-text)
		border-bottom-color: var(--c-accent)

.menu-wrapper
	position: relative
	flex-shrink: 0
	border-left: 1px solid var(--c-border)

.menu-button
	display: flex
	justify-content: center
	align-items: center
	padding: var(--sp-3) var(--sp-4)
	background: none
	border: none
	color: var(--c-text-sub)
	cursor: pointer

.menu-overlay
	position: fixed
	inset: 0
	background: transparent
	border: none
	cursor: default
	z-index: 9

.menu-dropdown
	position: absolute
	right: 0
	top: 100%
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-lg)
	box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25)
	z-index: 10
	min-width: 160px

.menu-item
	display: block
	width: 100%
	padding: var(--sp-3) var(--sp-4)
	color: var(--c-text-sub)
	text-decoration: none
	font-size: var(--fs-sm)
	white-space: nowrap
	text-align: left
	background: none
	border: none
	cursor: pointer
	font-family: inherit

	&:hover
		color: var(--c-text)
		background: var(--c-overlay-1)

	&.active
		color: var(--c-text)

	&:first-child
		border-radius: var(--radius-lg) var(--radius-lg) 0 0

	&:last-child
		border-radius: 0 0 var(--radius-lg) var(--radius-lg)

.menu-action
	&:disabled
		color: var(--c-text-muted)
		cursor: wait
		background: none

@media (max-width: 767px)
	.group-select
		display: block

	.nav-tabs
		display: none
</style>
