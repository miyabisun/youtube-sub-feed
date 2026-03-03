import config from '$lib/config.js';
import fetcher from '$lib/fetcher.js';

let groups = $state([]);
let loaded = false;

export async function loadGroups(force = false) {
	if (loaded && !force) return;
	try {
		groups = await fetcher(`${config.path.api}/groups`);
		loaded = true;
	} catch {}
}

export function setGroups(value) {
	groups = value;
	loaded = true;
}

export function getGroups() {
	return groups;
}
