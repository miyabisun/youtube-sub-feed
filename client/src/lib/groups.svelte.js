import config from '$lib/config.js';
import fetcher from '$lib/fetcher.js';

let groups = $state([]);

export async function loadGroups() {
	try {
		groups = await fetcher(`${config.path.api}/groups`);
	} catch {}
}

export function setGroups(value) {
	groups = value;
}

export function getGroups() {
	return groups;
}
