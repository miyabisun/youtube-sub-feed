<script>
	import { router, link, navigate } from '$lib/router.svelte.js';
	import config from '$lib/config.js';
	import fetcher from '$lib/fetcher.js';

	let groups = $state([]);

	async function loadGroups() {
		try {
			groups = await fetcher(`${config.path.api}/groups`);
		} catch {}
	}

	loadGroups();

	let selectValue = $state('');
	$effect(() => {
		selectValue = router.index === 1 ? router.params.id : '';
	});

	function isActive(href) {
		if (href === '/') return router.index === 0;
		if (href.startsWith('/group/')) return router.index === 1 && router.params.id === href.split('/')[2];
		if (href === '/settings') return router.index === 5;
		return false;
	}

	function onGroupSelect(e) {
		const value = e.target.value;
		navigate(value ? `/group/${value}` : '/');
	}
</script>

<header>
	<select class="group-select" bind:value={selectValue} onchange={onGroupSelect}>
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
	<nav class="nav-right">
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

@media (max-width: 799px)
	.group-select
		display: block

	.nav-tabs
		display: none
</style>
