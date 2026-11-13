export class Resource<T> {
	data = $state.raw<T | undefined>(undefined);
	error = $state.raw<unknown>(undefined);
	loading = $state(false);
	#seq = 0;
	readonly #fetch: () => Promise<T>;

	constructor(fetch: () => Promise<T>) {
		this.#fetch = fetch;
	}

	async load(): Promise<void> {
		const seq = ++this.#seq;
		this.loading = true;
		try {
			const data = await this.#fetch();
			if (seq !== this.#seq) return;
			this.data = data;
			this.error = undefined;
		} catch (e) {
			if (seq === this.#seq) this.error = e;
		} finally {
			if (seq === this.#seq) this.loading = false;
		}
	}

	set(data: T) {
		this.#seq++;
		this.data = data;
		this.error = undefined;
		this.loading = false;
	}
}

export interface Page<T> {
	items: T[];
	next: string | null;
}

/** A cursor-paginated list: `reload` replaces it with the first page, `more` appends the next. */
export class Paged<T> {
	items = $state.raw<T[]>([]);
	error = $state.raw<unknown>(undefined);
	loading = $state(false);
	loaded = $state(false);
	more = $state<'idle' | 'loading' | 'error' | 'done'>('done');
	#cursor: string | null = null;
	#seq = 0;
	readonly #fetch: (cursor: string | null) => Promise<Page<T>>;

	constructor(fetch: (cursor: string | null) => Promise<Page<T>>) {
		this.#fetch = fetch;
	}

	async reload(): Promise<void> {
		const seq = ++this.#seq;
		this.loading = true;
		try {
			const page = await this.#fetch(null);
			if (seq !== this.#seq) return;
			this.items = page.items;
			this.#cursor = page.next;
			this.more = page.next ? 'idle' : 'done';
			this.error = undefined;
			this.loaded = true;
		} catch (e) {
			if (seq === this.#seq) this.error = e;
		} finally {
			if (seq === this.#seq) this.loading = false;
		}
	}

	replace(match: (item: T) => boolean, next: T) {
		this.items = this.items.map((item) => (match(item) ? next : item));
	}

	async loadMore(): Promise<void> {
		if (!this.#cursor || this.more === 'loading') return;
		const seq = this.#seq;
		this.more = 'loading';
		try {
			const page = await this.#fetch(this.#cursor);
			if (seq !== this.#seq) return;
			this.items = [...this.items, ...page.items];
			this.#cursor = page.next;
			this.more = page.next ? 'idle' : 'done';
		} catch {
			if (seq === this.#seq) this.more = 'error';
		}
	}
}
