import { isModifiedClick } from '@dorsk/tsumikit';

type QueryPatch = Record<string, string | null | undefined>;

class Router {
	path = $state(window.location.pathname);
	search = $state(window.location.search);

	constructor() {
		window.addEventListener('popstate', () => this.#sync());
	}

	#sync() {
		this.path = window.location.pathname;
		this.search = window.location.search;
	}

	get query(): URLSearchParams {
		return new URLSearchParams(this.search);
	}

	get segments(): string[] {
		return this.path.split('/').filter(Boolean).map(decodeURIComponent);
	}

	navigate(href: string, options: { replace?: boolean } = {}) {
		const url = new URL(href, window.location.href);
		const samePage = url.pathname === this.path;
		if (url.href === window.location.href) return;
		window.history[options.replace ? 'replaceState' : 'pushState'](null, '', url);
		this.#sync();
		if (!samePage) window.scrollTo(0, 0);
	}

	/** The current page with some search params replaced; an empty value removes the param. */
	href(patch: QueryPatch): string {
		const query = this.query;
		for (const [key, value] of Object.entries(patch)) {
			if (value) query.set(key, value);
			else query.delete(key);
		}
		const search = query.toString();
		return search ? `${this.path}?${search}` : this.path;
	}

	setQuery(patch: QueryPatch) {
		this.navigate(this.href(patch), { replace: true });
	}
}

export const router = new Router();

/** Routes plain clicks on same-origin anchors through the router, so every link stays a real link. */
export function routeLinks(node: Document) {
	const onclick = (e: MouseEvent) => {
		if (e.defaultPrevented || isModifiedClick(e)) return;
		const anchor = (e.target as Element | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
		if (!anchor || anchor.target || anchor.hasAttribute('download')) return;
		const url = new URL(anchor.href);
		if (url.origin !== window.location.origin || url.pathname.startsWith('/v1/')) return;
		e.preventDefault();
		router.navigate(url.href);
	};
	node.addEventListener('click', onclick);
	return { destroy: () => node.removeEventListener('click', onclick) };
}

export const reportHref = (project: string, id: string) =>
	`/reports/${encodeURIComponent(project)}/${encodeURIComponent(id)}`;
