import { api } from './api.ts';
import { Resource } from './query.svelte.ts';

export const me = new Resource(api.me);
export const projects = new Resource(async () => (await api.projects()).projects);
