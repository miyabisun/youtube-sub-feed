export function getBasePath() {
	return (window.__BASE_PATH__ || '').replace(/\/+$/, '');
}

export function link(path) {
	return `${getBasePath()}${path}`;
}

let _routeIndex = $state(0);
let _params = $state({});

export const routes = [
	{ pattern: /^\/$/, params: [] },
	{ pattern: /^\/group\/([^/]+)$/, params: ['id'] },
	{ pattern: /^\/channels$/, params: [] },
	{ pattern: /^\/channel\/([^/]+)$/, params: ['id'] },
	{ pattern: /^\/login$/, params: [] },
	{ pattern: /^\/settings$/, params: [] },
];

export function matchRoute(path) {
	for (let i = 0; i < routes.length; i++) {
		const match = path.match(routes[i].pattern);
		if (match) {
			const params = {};
			routes[i].params.forEach((key, j) => {
				params[key] = decodeURIComponent(match[j + 1]);
			});
			return { index: i, params };
		}
	}
	return { index: 0, params: {} };
}

function getPathFromURL() {
	const base = getBasePath();
	let path = window.location.pathname;
	if (base && path.startsWith(base)) {
		path = path.slice(base.length) || '/';
	}
	return path;
}

function syncRoute() {
	const result = matchRoute(getPathFromURL());
	_routeIndex = result.index;
	_params = result.params;
}

export function navigate(path) {
	history.pushState({}, '', getBasePath() + path);
	syncRoute();
}

window.addEventListener('popstate', syncRoute);

// Initialize on load
syncRoute();

export const router = {
	get index() { return _routeIndex; },
	get params() { return _params; },
};
