import { navigate } from '$lib/router.svelte.js';

export default async function fetcher(url, options = {}) {
	const res = await fetch(url, options);
	if (res.status === 401) {
		navigate('/login');
		throw new Error('Unauthorized');
	}
	if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
	return res.json();
}
