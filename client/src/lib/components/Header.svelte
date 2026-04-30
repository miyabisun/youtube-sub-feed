<script>
	import { router, link, navigate } from '$lib/router.svelte.js';
	import { getGroups, loadGroups } from '$lib/groups.svelte.js';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';
	import Toast from '$lib/components/Toast.svelte';

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

	async function syncChannels() {
		if (syncing) return;
		syncing = true;
		closeMenu();
		try {
			const result = await fetcher(`${config.path.api}/channels/sync`, { method: 'POST' });
			const added = result?.added ?? 0;
			const removed = result?.removed ?? 0;
			toast = { message: `チャンネル同期完了 (追加: ${added}, 削除: ${removed})`, type: 'success' };
		} catch (e) {
			console.error('[sync] channel sync failed:', e);
			toast = { message: 'チャンネル同期に失敗しました', type: 'error' };
		} finally {
			syncing = false;
		}
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
			<span class="hamburger"></span>
			<span class="hamburger"></span>
			<span class="hamburger"></span>
		</button>
		{#if menuOpen}
			<button class="menu-overlay" onclick={closeMenu} aria-label="メニューを閉じる"></button>
			<nav class="menu-dropdown">
				<a class="menu-item" class:active={isActive('/channels')} href={link('/channels')} onclick={closeMenu}>チャンネル</a>
				<a class="menu-item" class:active={isActive('/settings')} href={link('/settings')} onclick={closeMenu}>グループ管理</a>
				<button class="menu-item menu-action" onclick={syncChannels} disabled={syncing}>
					{syncing ? '同期中...' : 'チャンネル同期'}
				</button>
				<a class="menu-item" href={`${config.path.api}/auth/login`} onclick={closeMenu}>再ログイン</a>
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
		outline: none
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
	flex-direction: column
	justify-content: center
	align-items: center
	gap: 4px
	padding: var(--sp-3) var(--sp-4)
	background: none
	border: none
	cursor: pointer

.hamburger
	display: block
	width: 18px
	height: 2px
	background: var(--c-text-sub)
	border-radius: 1px

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
	border-radius: var(--radius-md)
	box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15)
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
		border-radius: var(--radius-md) var(--radius-md) 0 0

	&:last-child
		border-radius: 0 0 var(--radius-md) var(--radius-md)

.menu-action
	&:disabled
		color: var(--c-text-muted)
		cursor: wait
		background: none

@media (max-width: 799px)
	.group-select
		display: block

	.nav-tabs
		display: none
</style>
