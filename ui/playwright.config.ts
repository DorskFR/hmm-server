import { defineConfig } from '@playwright/test';

const PORT = 4189;

export default defineConfig({
	testDir: 'tests',
	testMatch: '**/*.spec.ts',
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 1 : 0,
	reporter: process.env.CI ? 'github' : 'list',
	expect: { timeout: 10000 },
	workers: 1,
	use: {
		browserName: 'chromium',
		viewport: { width: 1440, height: 900 },
		baseURL: `http://127.0.0.1:${PORT}`,
	},
	webServer: {
		command: `npm run dev:mock -- --host 127.0.0.1`,
		env: { PORT: String(PORT) },
		url: `http://127.0.0.1:${PORT}/`,
		reuseExistingServer: !process.env.CI,
	},
});
