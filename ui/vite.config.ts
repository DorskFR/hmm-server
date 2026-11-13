import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';
import { mockApi } from './mock/plugin.ts';

const mock = process.env.HMM_MOCK === '1';

export default defineConfig({
	plugins: [svelte(), ...(mock ? [mockApi()] : [])],
	server: {
		port: Number(process.env.PORT ?? 5183),
		strictPort: true,
		proxy: mock ? undefined : { '/v1': 'http://127.0.0.1:8081' },
	},
	build: { outDir: 'dist', emptyOutDir: true },
});
