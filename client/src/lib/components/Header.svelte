<script>
	import { router, link } from '$lib/router.svelte.js';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';

	let groups = $state([]);

	async function loadGroups() {
		try {
			groups = await fetcher(`${config.path.api}/groups`);
		} catch {}
	}

	loadGroups();

	function isActive(href) {
		if (href === '/') return router.index === 0;
		if (href.startsWith('/group/')) return router.index === 1 && router.params.id === href.split('/')[2];
		if (href === '/channels') return router.index === 2;
		if (href === '/settings') return router.index === 5;
		return false;
	}
</script>

<header>
	<nav class="nav-tabs">
		<a class="nav-item" class:active={isActive('/')} href={link('/')}>すべて</a>
		{#each groups as group}
			<a class="nav-item" class:active={isActive(`/group/${group.id}`)} href={link(`/group/${group.id}`)}>{group.name}</a>
		{/each}
	</nav>
	<nav class="nav-right">
		<a class="nav-item" class:active={isActive('/channels')} href={link('/channels')}>チャンネル</a>
		<a class="nav-item" class:active={isActive('/settings')} href={link('/settings')}>設定</a>
	</nav>
</header>

<style lang="sass">
header
	display: flex
	align-items: center
	justify-content: space-between
	border-bottom: 1px solid var(--c-border)
	background: var(--c-bg)
	overflow-x: auto
	-webkit-overflow-scrolling: touch

.nav-tabs
	display: flex
	align-items: center
	overflow-x: auto
	flex: 1
	min-width: 0

	&::-webkit-scrollbar
		display: none

.nav-right
	display: flex
	align-items: center
	flex-shrink: 0
	border-left: 1px solid var(--c-border)

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
</style>
