<script>
	import { router, navigate, getBasePath } from '$lib/router.svelte.js';
	import config from '$lib/config.js';
	import Header from '$lib/components/Header.svelte';
	import Login from './pages/Login.svelte';
	import Feed from './pages/Feed.svelte';
	import Channels from './pages/Channels.svelte';
	import ChannelDetail from './pages/ChannelDetail.svelte';
	import Settings from './pages/Settings.svelte';

	let authenticated = $state(false);
	let checking = $state(true);

	async function checkAuth() {
		try {
			const res = await fetch(`${config.path.api}/auth/me`);
			authenticated = res.ok;
		} catch {
			authenticated = false;
		}
		checking = false;

		if (!authenticated && router.index !== 4) {
			navigate('/login');
		}
	}

	checkAuth();

	$effect(() => {
		function handleClick(e) {
			const a = e.target.closest('a');
			if (!a) return;
			const href = a.getAttribute('href');
			if (!href || href.startsWith('http') || href.startsWith('//')) return;
			if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;

			const base = getBasePath();
			let path = href;
			if (base && path.startsWith(base)) {
				path = path.slice(base.length) || '/';
			}

			e.preventDefault();
			navigate(path);
		}

		document.addEventListener('click', handleClick);
		return () => document.removeEventListener('click', handleClick);
	});
</script>

{#if checking}
	<div class="app loading"></div>
{:else}
	<div class="app">
		{#if router.index === 4}
			<main><Login /></main>
		{:else}
			<Header />
			<main>
				{#if router.index === 0}
					<Feed />
				{:else if router.index === 1}
					<Feed groupId={router.params.id} />
				{:else if router.index === 2}
					<Channels />
				{:else if router.index === 3}
					<ChannelDetail channelId={router.params.id} />
				{:else if router.index === 5}
					<Settings />
				{/if}
			</main>
		{/if}
	</div>
{/if}

<style lang="sass">
.app
	display: grid
	grid-template-rows: auto 1fr
	height: 100dvh

	&.loading
		display: flex
		align-items: center
		justify-content: center

main
	overflow-y: auto
</style>
